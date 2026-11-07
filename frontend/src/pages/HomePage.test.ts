import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FriendsOut } from "../api/generated";
import { activityItem, bookOut, clubOut, clubPick, favorite, post, shelfItem, user } from "../test/fixtures";
import { friendClubInvite, friendOut, friendRequest, friendsOut } from "../test/friends";
import { memoryStorage } from "../test/storage";
import type { ClubSummary } from "../types";

const api = {
  clubs: vi.fn(),
  clubPick: vi.fn(),
  addToShelf: vi.fn(),
  myShelf: vi.fn(),
  patchShelf: vi.fn(),
  diaryFeed: vi.fn(),
  activity: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status = 400, detail: unknown = message) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  },
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { heroClubSlug, LAST_CLUB_KEY } from "../stores/clubs";
import { useFinish } from "../stores/finish";
import { useFriends } from "../stores/friends";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import HomePage from "./HomePage.vue";

const SUMMARY: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "Europe/Berlin", member_count: 3 };
const OWLS: ClubSummary = { slug: "night-owls-cd34", name: "Night owls", role: "member", timezone: "UTC", member_count: 2 };

/** Home with the viewer's clubs and, when given, the friends lists the shell loaded. */
async function mountHome(clubs: (typeof SUMMARY)[], friends?: FriendsOut) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  if (friends) useFriends().take(friends);
  const wrapper = mount(HomePage, { global: { plugins: [pinia, i18n], stubs: { BookCover: true, transition: true } } });
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
    api.clubPick.mockImplementation(async (slug: string) => ({
      pick: clubPick({ book: bookOut({ id: 1, title: "Circe" }), on_shelf: "currently_reading", shelf_id: 1 }),
      timezone: slug === OWLS.slug ? "UTC" : "Europe/Berlin",
    }));
    vi.stubGlobal("localStorage", memoryStorage());
    api.activity.mockResolvedValue({
      items: [activityItem({ kind: "nominated", book: bookOut({ title: "Circe" }), club: { slug: "readers-ab12", name: "Readers" } })],
      has_more: false,
      timezone: "UTC",
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("makes no club call in solo mode and puts the book I read at the centre", async () => {
    const wrapper = await mountHome([]);
    expect(api.clubs).not.toHaveBeenCalled();
    expect(api.clubPick).not.toHaveBeenCalled();
    expect(api.activity).not.toHaveBeenCalled();
    // One quiet line to create or join a club.
    expect(wrapper.get(".solo-prompt").text()).toContain("Read together?");
    expect(wrapper.find(".solo-prompt a[href='/clubs/new']").exists()).toBe(true);
    expect(wrapper.find(".solo-prompt a[href='/clubs']").exists()).toBe(true);
    // The greeting stands alone: the section headings say what follows.
    expect(wrapper.find(".page-head .lede").exists()).toBe(false);
    // The hero is my top book being read; the list holds only the others, so it goes.
    expect(wrapper.get(".home-hero .hero-title").text()).toBe("Circe");
    expect(wrapper.find(".home-hero .hero-club").exists()).toBe(false);
    expect(wrapper.find(".home-reading").exists()).toBe(false);
    expect(wrapper.findAll(".progress-trigger")).toHaveLength(1);
    // Notes in short, and my favourites.
    expect(api.diaryFeed).toHaveBeenCalledWith(undefined, 4, undefined);
    expect(wrapper.text()).toContain("Middlemarch");
  });

  it("keeps the other books being read in the list below the solo hero", async () => {
    api.myShelf.mockResolvedValue({
      user: { id: 1, username: "ada", avatar_url: null },
      timezone: "UTC",
      items: [
        shelfItem({ id: 2, status: "currently_reading", position: 1, book: { title: "Later" } }),
        shelfItem({ id: 1, status: "currently_reading", position: 0, progress: 30, book: { title: "Circe" } }),
      ],
      favorites: [],
    });
    const wrapper = await mountHome([]);
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    expect(wrapper.findAll(".reading-title").map((row) => row.text())).toEqual(["Later"]);
  });

  it("shows nothing-read with a way to a book when the solo shelf has none", async () => {
    api.myShelf.mockResolvedValue({ user: { id: 1, username: "ada", avatar_url: null }, timezone: "UTC", items: [], favorites: [] });
    const wrapper = await mountHome([]);
    expect(wrapper.find(".home-hero").exists()).toBe(false);
    expect(wrapper.get(".reading-empty").text()).toContain("You're not reading anything.");
  });

  it("puts the club's book at the centre, with the activity and the diary in short", async () => {
    api.clubs.mockResolvedValue({
      items: [clubOut({ ...SUMMARY, current_pick: { id: 3, book: bookOut({ id: 1, title: "Circe" }), meeting_at: null } })],
    });
    api.activity.mockResolvedValue({
      items: [activityItem({ kind: "nominated", book: bookOut({ title: "Circe" }), club: { slug: "readers-ab12", name: "Readers" } })],
      has_more: true,
      timezone: "UTC",
    });
    api.diaryFeed.mockResolvedValue({ items: [], has_more: false, timezone: "UTC" });
    const wrapper = await mountHome([SUMMARY]);
    expect(api.clubs).toHaveBeenCalledTimes(1);
    expect(api.clubPick).toHaveBeenCalledTimes(1);
    expect(api.clubPick).toHaveBeenCalledWith("readers-ab12");
    const hero = wrapper.get(".home-hero");
    expect(hero.get(".hero-club").attributes("href")).toBe("/c/readers-ab12");
    expect(hero.get(".hero-club").text()).toContain("Readers");
    expect(hero.get(".hero-title").text()).toBe("Circe");
    // One club: no chips.
    expect(hero.find(".hero-others").exists()).toBe(false);
    // I read the club's book: its progress is in the hero, not again in the list.
    expect(hero.find(".progress-trigger").exists()).toBe(true);
    expect(wrapper.findAll(".progress-trigger")).toHaveLength(1);
    expect(wrapper.find(".home-reading").exists()).toBe(false);
    expect(api.activity).toHaveBeenCalledWith({ before: undefined, limit: 5, club: undefined });
    expect(wrapper.get(".activity-row").text()).toContain("ada nominated Circe");
    expect(wrapper.get(".activity-club").attributes("href")).toBe("/c/readers-ab12");
    // The club's events page leaves out my own events from before I joined: Home pages in place.
    expect(wrapper.find(".activity-all").exists()).toBe(false);
    expect(wrapper.find(".activity-more").exists()).toBe(true);
    expect(api.diaryFeed).toHaveBeenCalledWith(undefined, 4, undefined);
    expect(wrapper.find(".solo-prompt").exists()).toBe(false);
  });

  it("picks the hero club: the remembered one, else the soonest meeting", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-06T12:00:00Z"));
    const pick = (id: number, meeting_at: string | null) => ({ id, book: bookOut({ id: 10 + id }), meeting_at });
    const list = [
      clubOut({ ...SUMMARY, current_pick: pick(1, "2026-11-20T18:00:00Z") }),
      clubOut({ ...OWLS, current_pick: pick(2, "2026-11-12T18:00:00Z") }),
    ];
    expect(heroClubSlug(list, null)).toBe(OWLS.slug);
    expect(heroClubSlug(list, SUMMARY.slug)).toBe(SUMMARY.slug);
    // A meeting already past does not count; then the first club with a book, then the newest join.
    list[1]!.current_pick!.meeting_at = "2026-11-01T18:00:00Z";
    list[0]!.current_pick!.meeting_at = null;
    expect(heroClubSlug(list, null)).toBe(SUMMARY.slug);
    expect(heroClubSlug([clubOut({ ...SUMMARY }), clubOut({ ...OWLS, current_pick: pick(2, null) })], null)).toBe(OWLS.slug);
    expect(heroClubSlug([clubOut({ ...SUMMARY }), clubOut({ ...OWLS })], null)).toBe(SUMMARY.slug);

    api.clubs.mockResolvedValue({
      items: [
        clubOut({ ...SUMMARY, current_pick: pick(1, "2026-11-20T18:00:00Z") }),
        clubOut({ ...OWLS, current_pick: pick(2, "2026-11-12T18:00:00Z") }),
      ],
    });
    let wrapper = await mountHome([SUMMARY, OWLS]);
    expect(api.clubPick).toHaveBeenCalledTimes(1);
    expect(api.clubPick).toHaveBeenLastCalledWith(OWLS.slug);
    expect(wrapper.findAll(".hero-chip").map((chip) => chip.text())).toEqual(["Readers"]);

    api.clubPick.mockClear();
    localStorage.setItem(LAST_CLUB_KEY, SUMMARY.slug);
    wrapper = await mountHome([SUMMARY, OWLS]);
    expect(api.clubPick).toHaveBeenCalledTimes(1);
    expect(api.clubPick).toHaveBeenLastCalledWith(SUMMARY.slug);
    expect(wrapper.findAll(".hero-chip").map((chip) => chip.text())).toEqual(["Night owls"]);
  });

  it("swaps the hero in place from a chip and remembers that club", async () => {
    api.clubs.mockResolvedValue({ items: [clubOut({ ...SUMMARY }), clubOut({ ...OWLS })] });
    api.clubPick.mockImplementation(async (slug: string) => ({
      pick: clubPick({ book: bookOut({ id: slug === OWLS.slug ? 20 : 1, title: slug === OWLS.slug ? "Owl book" : "Circe" }) }),
      timezone: "UTC",
    }));
    const wrapper = await mountHome([SUMMARY, OWLS]);
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    await wrapper.get(".hero-chip").trigger("click");
    await flushPromises();
    expect(api.clubPick).toHaveBeenLastCalledWith(OWLS.slug);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe(OWLS.slug);
    expect(wrapper.get(".hero-title").text()).toBe("Owl book");
    expect(wrapper.get(".hero-club").text()).toContain("Night owls");
    expect(wrapper.findAll(".hero-chip").map((chip) => chip.text())).toEqual(["Readers"]);
    // Circe is mine to read and no longer the hero's: it is back in the list.
    expect(wrapper.findAll(".reading-title").map((row) => row.text())).toEqual(["Circe"]);
  });

  it("finishes another club's pick from its row with that club's line in the moment", async () => {
    const owlBook = bookOut({ id: 20, title: "Owl book", pages: null });
    api.clubs.mockResolvedValue({
      items: [
        clubOut({ ...SUMMARY, current_pick: { id: 1, book: bookOut({ id: 1, title: "Circe" }), meeting_at: null } }),
        clubOut({ ...OWLS, current_pick: { id: 2, book: owlBook, meeting_at: null } }),
      ],
    });
    api.myShelf.mockResolvedValue({
      user: { id: 1, username: "ada", avatar_url: null },
      timezone: "UTC",
      items: [
        shelfItem({ id: 1, status: "currently_reading", position: 0, progress: 30, book: { id: 1, title: "Circe" } }),
        shelfItem({ id: 3, status: "currently_reading", position: 1, progress: 90, book: owlBook }),
      ],
      favorites: [],
    });
    localStorage.setItem(LAST_CLUB_KEY, SUMMARY.slug);
    const wrapper = await mountHome([SUMMARY, OWLS]);
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    const row = wrapper.get(".reading-row");
    expect(row.get(".reading-title").text()).toBe("Owl book");
    await row.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("100");
    const finished = shelfItem({ id: 3, status: "finished", book: owlBook });
    api.patchShelf.mockResolvedValue(finished);
    await wrapper.findAll(".sheet button").find((button) => button.text() === "Mark as read")!.trigger("click");
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(3, { status: "finished", position: 0 });
    expect(useFinish().item).toEqual(finished);
    expect(useFinish().clubSlug).toBe(OWLS.slug);
  });

  it("pages the diary in place: no page holds Home's feed with friends' notes", async () => {
    const note = { parent_author: null, my_progress: null, my_status: null, club: null, book: bookOut({ id: 1 }), entry: post({ id: 11, author: "ada" }) };
    api.diaryFeed.mockResolvedValue({ items: [note], has_more: true, timezone: "UTC" });
    for (const friends of [friendsOut(), friendsOut({ friends: [friendOut("grace")] })]) {
      const wrapper = await mountHome([], friends);
      expect(wrapper.find(".feed-all").exists()).toBe(false);
      expect(wrapper.find(".feed-load-more").exists()).toBe(true);
    }
  });

  it("moves on to the next club when the hero's club is gone", async () => {
    api.clubs.mockResolvedValue({ items: [clubOut({ ...SUMMARY }), clubOut({ ...OWLS })] });
    api.clubPick.mockImplementation(async (slug: string) => {
      if (slug === SUMMARY.slug) throw new ApiError("No such club", 404, "No such club");
      return { pick: clubPick({ book: bookOut({ id: 20, title: "Owl book" }) }), timezone: "UTC" };
    });
    const wrapper = await mountHome([SUMMARY, OWLS]);
    expect(api.clubPick.mock.calls.map(([slug]) => slug)).toEqual([SUMMARY.slug, OWLS.slug]);
    expect(wrapper.get(".hero-title").text()).toBe("Owl book");
    expect(wrapper.find(".hero-others").exists()).toBe(false);
  });

  it("shows friends' events to a viewer with friends and no club", async () => {
    api.activity.mockResolvedValue({
      items: [activityItem({ kind: "shelf_finished", actor: "grace", book: bookOut({ title: "The Hobbit" }), club: null })],
      has_more: false,
      timezone: "UTC",
    });
    const wrapper = await mountHome([], friendsOut({ friends: [friendOut("grace")] }));
    expect(api.clubs).not.toHaveBeenCalled();
    expect(api.activity).toHaveBeenCalledWith({ before: undefined, limit: 5, club: undefined });
    expect(wrapper.get(".activity-row").text()).toContain("grace finished The Hobbit");
    expect(wrapper.find(".activity-club").exists()).toBe(false);
    // Still solo: the prompt to start or join a club, and the favourites.
    expect(wrapper.find(".solo-prompt").exists()).toBe(true);
    expect(wrapper.text()).toContain("Middlemarch");
  });

  it("shows a row to the friends page only while requests or invitations wait there", async () => {
    // Before the shell's lists arrive (or when they failed), and with nothing waiting: no row.
    let wrapper = await mountHome([]);
    expect(wrapper.find(".friends-row").exists()).toBe(false);
    wrapper = await mountHome([], friendsOut());
    expect(wrapper.find(".friends-row").exists()).toBe(false);
    wrapper = await mountHome([], friendsOut({ friends: [friendOut("grace")], outgoing: [friendRequest("olga")] }));
    expect(wrapper.find(".friends-row").exists()).toBe(false);

    wrapper = await mountHome(
      [SUMMARY],
      friendsOut({
        friends: [friendOut("grace")],
        incoming: [friendRequest("mia"), friendRequest("max")],
        outgoing: [friendRequest("olga")],
        club_invites: [friendClubInvite()],
      }),
    );
    const row = wrapper.get("a.friends-row");
    expect(row.attributes("href")).toBe("/friends");
    expect(row.get(".friends-row-label").text()).toBe("3 waiting for you");

    wrapper = await mountHome([], friendsOut({ club_invites: [friendClubInvite()] }));
    expect(wrapper.get(".friends-row-label").text()).toBe("1 waiting for you");
    // Home asks the server nothing itself: the shell loads the lists.
    expect(Object.keys(api)).not.toContain("friends");
  });

  it("saves progress from the solo hero only on Save, and puts it back with Undo", async () => {
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

  it("reads the feed again when my position moves, so its spoiler guards follow it", async () => {
    const ahead = (myProgress: number) => ({
      parent_author: null,
      my_progress: myProgress,
      my_status: "currently_reading" as const,
      club: null,
      book: bookOut({ id: 1, title: "Circe" }),
      entry: post({ id: 11, author: "jonas", body: "The witch turns them.", spoiler_upto: null, progress_at: 50 }),
    });
    api.diaryFeed.mockResolvedValue({ items: [ahead(30)], has_more: false, timezone: "UTC" });
    const wrapper = await mountHome([]);
    expect(wrapper.get(".feed-guard-line").text()).toContain("you're at 30%");

    // The hero saves 60 %: the post is no longer ahead of me.
    api.diaryFeed.mockResolvedValue({ items: [ahead(60)], has_more: false, timezone: "UTC" });
    api.patchShelf.mockResolvedValue(shelfItem({ id: 1, status: "currently_reading", progress: 60 }));
    await wrapper.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("60");
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.diaryFeed).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".feed-guard").exists()).toBe(false);
    expect(wrapper.get(".feed-body").text()).toContain("The witch turns them.");

    // Undo back to 30 %: it folds again instead of staying open.
    api.diaryFeed.mockResolvedValue({ items: [ahead(30)], has_more: false, timezone: "UTC" });
    api.patchShelf.mockResolvedValue(shelfItem({ id: 1, status: "currently_reading", progress: 30 }));
    const toast = useToast();
    await toast.run(toast.items.find((row) => row.message === "Progress: 60%")!.id);
    await flushPromises();
    expect(api.diaryFeed).toHaveBeenCalledTimes(3);
    expect(wrapper.get(".feed-guard-line").text()).toContain("you're at 30%");
  });

  it("reads the feed again after a row's save, but not for stars alone", async () => {
    api.myShelf.mockResolvedValue({
      user: { id: 1, username: "ada", avatar_url: null },
      timezone: "UTC",
      items: [
        shelfItem({ id: 1, status: "currently_reading", position: 0, progress: 30, book: { id: 1, title: "Circe" } }),
        shelfItem({ id: 2, status: "currently_reading", position: 1, progress: 10, book: { id: 2, title: "Later", pages: null } }),
      ],
      favorites: [],
    });
    const wrapper = await mountHome([]);
    expect(api.diaryFeed).toHaveBeenCalledTimes(1);
    const row = wrapper.get(".reading-row");
    await row.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("40");
    api.patchShelf.mockResolvedValue(
      shelfItem({ id: 2, status: "currently_reading", position: 1, progress: 40, book: { id: 2, title: "Later", pages: null } }),
    );
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(2, { progress: 40 });
    expect(api.diaryFeed).toHaveBeenCalledTimes(2);
    // The moment saved stars: the position did not move, the feed stays.
    useFinish().lastSaved = shelfItem({ id: 2, status: "currently_reading", position: 1, progress: 40, rating: 4, book: { id: 2, title: "Later", pages: null } });
    await flushPromises();
    expect(api.diaryFeed).toHaveBeenCalledTimes(2);
  });
});
