import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { post, user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api = {
  diary: vi.fn(),
  addDiaryEntry: vi.fn(),
  pickThread: vi.fn(),
  addPickPost: vi.fn(),
  editDiaryEntry: vi.fn(),
  deleteDiaryEntry: vi.fn(),
  reactToEntry: vi.fn(),
};

vi.mock("../api/client", () => {
  class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status: number, detail: unknown) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import BookDiary from "./BookDiary.vue";

const CLUB = { slug: "readers-ab12", name: "Readers" };

async function mountDiary(props: Record<string, unknown>, role: string | null = "member") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({
    clubs: role ? [{ ...CLUB, role, timezone: "Europe/Berlin", member_count: 3 }] : [],
  });
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/", component: { template: "<p />" } }] });
  await router.push("/");
  const wrapper = mount(BookDiary, { props, global: { plugins: [pinia, router, i18n] } });
  await flushPromises();
  return wrapper;
}

function thread(overrides: Record<string, unknown> = {}) {
  return { pick_id: 5, can_post: true, my_progress: 40, my_status: "currently_reading", items: [], timezone: "Europe/Berlin", ...overrides };
}

describe("BookDiary", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("shows a club pick's discussion and posts into it", async () => {
    api.pickThread.mockResolvedValue(thread());
    const wrapper = await mountDiary({ club: { slug: CLUB.slug, pickId: null } });
    expect(api.pickThread).toHaveBeenCalledWith(CLUB.slug, null);
    expect(api.diary).not.toHaveBeenCalled();
    // Club posts are never private, so there is no "only me" here.
    expect(wrapper.find(".only-me").exists()).toBe(false);

    api.addPickPost.mockResolvedValue(post({ id: 8, body: "Chapter 3!", club: CLUB, pick_id: 5, mine: true }));
    await wrapper.get(".compose textarea").setValue("Chapter 3!");
    await wrapper.get(".compose").trigger("submit");
    await flushPromises();
    expect(api.addPickPost).toHaveBeenCalledWith(CLUB.slug, null, { body: "Chapter 3!", spoiler_upto: 40 });
    expect(wrapper.text()).toContain("Chapter 3!");
    // The club page refreshes its feed of members' writing on this.
    expect(wrapper.emitted("posted")?.[0]?.[0]).toMatchObject({ id: 8 });
    // In its own discussion a club post needs no club tag.
    expect(wrapper.find(".entry-club").exists()).toBe(false);
  });

  it("names the club of a club post in a work's diary", async () => {
    const items = [
      post({ id: 1, author: "grace", body: "Mine to keep" }),
      post({ id: 2, author: "max", body: "For Thursday", club: CLUB, pick_id: 5 }),
    ];
    api.diary.mockResolvedValue({ work_id: 9, my_progress: null, my_status: null, items, timezone: "UTC" });
    const wrapper = await mountDiary({ workId: 9 });
    const tags = wrapper.findAll(".entry-club");
    expect(tags).toHaveLength(1);
    expect(tags[0].text()).toBe("Readers");
    expect(tags[0].attributes("href")).toBe("/c/readers-ab12/discussions");
  });

  it("leaves the meeting out of a solo reader's prompts", async () => {
    api.diary.mockResolvedValue({ work_id: 9, my_progress: null, my_status: null, items: [], timezone: "UTC" });
    const solo = await mountDiary({ workId: 9 }, null);
    expect(solo.get(".diary-empty").text()).not.toContain("meeting");
    expect(solo.get(".compose textarea").attributes("placeholder")).not.toContain("meeting");
    const member = await mountDiary({ workId: 9 });
    expect(member.get(".diary-empty").text()).toContain("meeting");
  });

  it("keeps an ended pick's discussion read-only", async () => {
    api.pickThread.mockResolvedValue(
      thread({ can_post: false, items: [post({ id: 3, body: "Old talk", club: CLUB, pick_id: 2 })] }),
    );
    const wrapper = await mountDiary({ club: { slug: CLUB.slug, pickId: 2 } });
    expect(api.pickThread).toHaveBeenCalledWith(CLUB.slug, 2);
    expect(wrapper.text()).toContain("Old talk");
    expect(wrapper.find(".compose").exists()).toBe(false);
    expect(wrapper.findAll(".entry-links .text-btn").map((button) => button.text())).not.toContain("Reply");
    expect(wrapper.text()).toContain("This pick is over");
  });

  it("says so when the club has no pick to discuss", async () => {
    api.pickThread.mockRejectedValue(new ApiError("There is no club pick yet", 404, "No club pick yet"));
    const wrapper = await mountDiary({ club: { slug: CLUB.slug, pickId: null } });
    expect(wrapper.find(".error").exists()).toBe(false);
    expect(wrapper.text()).toContain("No book is picked right now");
    expect(wrapper.find(".compose").exists()).toBe(false);
  });

  it("keeps a diary entry to its author when asked, but never a reply", async () => {
    const root = post({ id: 1, author: "grace", body: "First" });
    api.diary.mockResolvedValue({ work_id: 9, my_progress: null, my_status: null, items: [root], timezone: "UTC" });
    const wrapper = await mountDiary({ workId: 9 });

    api.addDiaryEntry.mockResolvedValue(post({ id: 2, body: "Mine", mine: true, visibility: "only_me" }));
    await wrapper.get(".compose textarea").setValue("Mine");
    await wrapper.get(".only-me input").setValue(true);
    await wrapper.get(".compose").trigger("submit");
    await flushPromises();
    expect(api.addDiaryEntry).toHaveBeenLastCalledWith(9, { body: "Mine", spoiler_upto: null, visibility: "only_me" });
    expect(wrapper.get(".only-me-badge").text()).toBe("Only you");

    api.addDiaryEntry.mockResolvedValue(post({ id: 3, body: "A reply", parent_id: 1, mine: true }));
    await wrapper.findAll(".entry-links .text-btn").find((button) => button.text() === "Reply")!.trigger("click");
    await wrapper.get(".reply-form textarea").setValue("A reply");
    await wrapper.get(".reply-form").trigger("submit");
    await flushPromises();
    expect(api.addDiaryEntry).toHaveBeenLastCalledWith(9, { body: "A reply", spoiler_upto: null, parent_id: 1 });
  });

  it("lets a club admin delete another member's club post", async () => {
    const items = [post({ id: 4, author: "max", body: "Off topic", club: CLUB, pick_id: 5 })];
    api.pickThread.mockResolvedValue(thread({ items }));
    const asMember = await mountDiary({ club: { slug: CLUB.slug, pickId: null } }, "member");
    expect(asMember.findAll(".entry-links .text-btn").map((button) => button.text())).not.toContain("Delete");
    const asAdmin = await mountDiary({ club: { slug: CLUB.slug, pickId: null } }, "admin");
    const labels = asAdmin.findAll(".entry-links .text-btn").map((button) => button.text());
    expect(labels).toContain("Delete");
    expect(labels).not.toContain("Edit");
  });
});
