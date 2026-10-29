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
