import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { activityItem, bookOut, clubOut, favorite, shelfItem, user } from "../test/fixtures";

const api = {
  clubs: vi.fn(),
  myShelf: vi.fn(),
  patchShelf: vi.fn(),
  diaryFeed: vi.fn(),
  activity: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import HomePage from "./HomePage.vue";

const SUMMARY = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "Europe/Berlin", member_count: 3 };

async function mountHome(clubs: (typeof SUMMARY)[]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  const wrapper = mount(HomePage, { global: { plugins: [pinia, i18n], stubs: { BookCover: true } } });
  await flushPromises();
  return wrapper;
}

describe("HomePage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.myShelf.mockResolvedValue({
      user: { id: 1, username: "ada", avatar_url: null },
      timezone: "UTC",
      items: [
        shelfItem({ id: 1, status: "currently_reading", progress: 30, book: { title: "Circe" } }),
        shelfItem({ id: 2, status: "want_to_read", book: { title: "Later" } }),
      ],
      favorites: [favorite(1, { title: "Middlemarch" })],
    });
    api.diaryFeed.mockResolvedValue({ items: [], has_more: false, timezone: "UTC" });
    api.activity.mockResolvedValue({
      items: [activityItem({ kind: "nominated", book: bookOut({ title: "Circe" }), club: { slug: "readers-ab12", name: "Readers" } })],
      has_more: false,
      timezone: "UTC",
    });
  });

  it("makes no club call in solo mode and offers to create or join a club", async () => {
    const wrapper = await mountHome([]);
    expect(api.clubs).not.toHaveBeenCalled();
    expect(api.activity).not.toHaveBeenCalled();
    expect(wrapper.find(".solo-prompt").exists()).toBe(true);
    expect(wrapper.find(".solo-prompt a[href='/clubs/new']").exists()).toBe(true);
    // Personal things still work: what I'm reading, my notes, my favourites.
    expect(wrapper.findAll(".reading-title").map((row) => row.text())).toEqual(["Circe"]);
    expect(api.diaryFeed).toHaveBeenCalledWith(undefined, 8, undefined);
    expect(wrapper.text()).toContain("Middlemarch");
  });

  it("shows club cards, the combined activity feed and the combined diary", async () => {
    api.clubs.mockResolvedValue({
      items: [clubOut({ ...SUMMARY, current_pick: { id: 3, book: bookOut({ title: "Circe" }), meeting_at: null } })],
    });
    const wrapper = await mountHome([SUMMARY]);
    expect(api.clubs).toHaveBeenCalledTimes(1);
    const card = wrapper.get(".club-card");
    expect(card.attributes("href")).toBe("/c/readers-ab12");
    expect(card.text()).toContain("Readers");
    expect(card.text()).toContain("Reading Circe");
    expect(api.activity).toHaveBeenCalledWith({ before: undefined, limit: 15, club: undefined });
    expect(wrapper.get(".activity-row").text()).toContain("ada nominated Circe");
    expect(wrapper.get(".activity-club").attributes("href")).toBe("/c/readers-ab12");
    expect(api.diaryFeed).toHaveBeenCalledWith(undefined, 8, undefined);
    expect(wrapper.find(".solo-prompt").exists()).toBe(false);
  });

  it("saves progress from the reading list", async () => {
    const wrapper = await mountHome([]);
    api.patchShelf.mockResolvedValue(shelfItem({ id: 1, status: "currently_reading", progress: 55 }));
    const slider = wrapper.get("input[type=range]");
    await slider.setValue("55");
    await slider.trigger("change");
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(1, { progress: 55 });
  });
});
