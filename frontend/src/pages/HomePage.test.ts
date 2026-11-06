import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FriendsOut } from "../api/generated";
import { activityItem, bookOut, clubOut, favorite, shelfItem, user } from "../test/fixtures";
import { friendClubInvite, friendOut, friendRequest, friendsOut } from "../test/friends";
import type { ClubSummary } from "../types";

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
import { useFriends } from "../stores/friends";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import HomePage from "./HomePage.vue";

const SUMMARY: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "Europe/Berlin", member_count: 3 };

/** Home with the viewer's clubs and, when given, the friends lists the shell loaded. */
async function mountHome(clubs: (typeof SUMMARY)[], friends?: FriendsOut) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  if (friends) useFriends().take(friends);
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

  it("shows friends' events to a viewer with friends and no club", async () => {
    api.activity.mockResolvedValue({
      items: [activityItem({ kind: "shelf_finished", actor: "grace", book: bookOut({ title: "The Hobbit" }), club: null })],
      has_more: false,
      timezone: "UTC",
    });
    const wrapper = await mountHome([], friendsOut({ friends: [friendOut("grace")] }));
    expect(api.clubs).not.toHaveBeenCalled();
    expect(api.activity).toHaveBeenCalledWith({ before: undefined, limit: 15, club: undefined });
    expect(wrapper.get(".activity-row").text()).toContain("grace finished The Hobbit");
    expect(wrapper.find(".activity-club").exists()).toBe(false);
    // Still solo: the prompt to start or join a club, and the favourites.
    expect(wrapper.find(".solo-prompt").exists()).toBe(true);
    expect(wrapper.text()).toContain("Middlemarch");
  });

  it("leads to the friends page with what waits there, or the way to add friends", async () => {
    // Before the shell's lists arrive (or when they failed): the plain row.
    let wrapper = await mountHome([]);
    let row = wrapper.get("a.friends-row");
    expect(row.attributes("href")).toBe("/friends");
    expect(row.get(".friends-row-label").text()).toBe("Friends");
    expect(row.find(".friends-count").exists()).toBe(false);
    expect(row.text()).not.toContain("Add friends");

    wrapper = await mountHome([], friendsOut());
    expect(wrapper.get("a.friends-row").text()).toContain("Add friends");

    wrapper = await mountHome(
      [SUMMARY],
      friendsOut({
        friends: [friendOut("grace")],
        incoming: [friendRequest("mia"), friendRequest("max")],
        outgoing: [friendRequest("olga")],
        club_invites: [friendClubInvite()],
      }),
    );
    row = wrapper.get("a.friends-row");
    expect(row.get(".friends-count [aria-hidden=true]").text()).toBe("3");
    expect(row.get(".friends-count .visually-hidden").text()).toBe("3 requests or invitations waiting");
    expect(row.text()).not.toContain("Add friends");

    // Friends and nothing waiting: just the way there. Home asks the server nothing itself.
    wrapper = await mountHome([], friendsOut({ friends: [friendOut("grace")] }));
    row = wrapper.get("a.friends-row");
    expect(row.find(".friends-count").exists()).toBe(false);
    expect(row.text()).not.toContain("Add friends");
    expect(Object.keys(api)).not.toContain("friends");
  });

  it("saves progress from the reading list only on Save, and puts it back with Undo", async () => {
    const wrapper = await mountHome([]);
    // No slider on the page itself: a stray touch used to save and post to the club.
    expect(wrapper.find("input[type=range]").exists()).toBe(false);
    await wrapper.get(".progress-trigger").trigger("click");
    const slider = wrapper.get(".sheet input[type=range]");
    await slider.setValue("55");
    expect(api.patchShelf).not.toHaveBeenCalled();
    api.patchShelf.mockResolvedValue(shelfItem({ id: 1, status: "currently_reading", progress: 55 }));
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(1, { progress: 55 });
    expect(wrapper.get(".progress-trigger").text()).toContain("55%");

    const toast = useToast();
    const saved = toast.items.find((row) => row.message === "Progress: 55%");
    expect(saved?.action?.label).toBe("Undo");
    api.patchShelf.mockResolvedValue(shelfItem({ id: 1, status: "currently_reading", progress: 30 }));
    await toast.run(saved!.id);
    await flushPromises();
    expect(api.patchShelf).toHaveBeenLastCalledWith(1, { progress: 30 });
    expect(wrapper.get(".progress-trigger").text()).toContain("30%");
  });
});
