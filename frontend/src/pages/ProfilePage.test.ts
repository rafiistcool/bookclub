import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { FriendsOut } from "../api/generated";
import { bookOut, clubOut, favorite, shelfItem, user } from "../test/fixtures";
import { friendClubInvite, friendOut, friendRequest, friendsOut, reading } from "../test/friends";
import type { ClubSummary } from "../types";

const api = {
  myShelf: vi.fn(),
  personalStats: vi.fn(),
  clubs: vi.fn(),
  uploadAvatar: vi.fn(),
  deleteAvatar: vi.fn(),
  replaceFavorites: vi.fn(),
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
import { useFriends } from "../stores/friends";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import ProfilePage from "./ProfilePage.vue";

const READERS: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };
const OWLS: ClubSummary = { slug: "night-owls-cd34", name: "Night owls", role: "admin", timezone: "UTC", member_count: 1 };

let wrapper: VueWrapper | null = null;

async function mountProfile(options: { clubs?: ClubSummary[]; friends?: FriendsOut; avatar?: string | null } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs: options.clubs ?? [], avatar_url: options.avatar ?? null });
  if (options.friends) useFriends().take(options.friends);
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/:rest(.*)*", component: { template: "<p />" } }],
  });
  await router.push("/me");
  wrapper = mount(ProfilePage, {
    attachTo: document.body,
    global: { plugins: [pinia, router, i18n], stubs: { BookCover: { props: ["book"], template: '<span class="cover-stub">{{ book.title }}</span>' } } },
  });
  await flushPromises();
  return wrapper;
}

function toasts(): string[] {
  return useToast().items.map((toast) => toast.message);
}

describe("ProfilePage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
    api.myShelf.mockResolvedValue({
      user: { id: 1, username: "ada", avatar_url: null },
      timezone: "UTC",
      items: [shelfItem({ id: 1 }), shelfItem({ id: 2 }), shelfItem({ id: 3 })],
      favorites: [favorite(1, { id: 7, title: "Middlemarch" }), favorite(2, { id: 8, title: "Circe" })],
    });
    api.personalStats.mockResolvedValue({ username: "ada", year: 2026, years: [2026], finished: 12 });
    api.clubs.mockResolvedValue({
      items: [
        clubOut({ ...READERS, current_pick: { id: 1, book: bookOut({ id: 9, title: "Ulysses" }), meeting_at: null } }),
        clubOut({ ...OWLS }),
      ],
    });
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    document.body.innerHTML = "";
  });

  it("shows the reader, three numbers that lead on, the favourites, the friends and the clubs", async () => {
    const page = await mountProfile({
      clubs: [READERS, OWLS],
      friends: friendsOut({
        friends: [friendOut("grace", { currently_reading_count: 1, currently_reading: [reading(3)] }), friendOut("hal")],
        incoming: [friendRequest("mia")],
        club_invites: [friendClubInvite()],
      }),
    });
    expect(page.get("h1").text()).toBe("ada");
    expect(page.get(".profile-settings").attributes("href")).toBe("/settings");
    expect(page.get(".profile-settings").attributes("aria-label")).toBe("Settings");
    expect(api.personalStats).toHaveBeenCalledWith({});

    const numbers = page.findAll(".number");
    expect(numbers.map((link) => link.attributes("href"))).toEqual(["/stats", "/shelf", "/friends"]);
    expect(numbers.map((link) => link.get(".number-value").text())).toEqual(["12", "3", "2"]);
    expect(numbers.map((link) => link.get(".number-label").text())).toEqual(["Read in 2026", "On your shelf", "Friends"]);

    expect(page.findAll(".portrait .cover-stub").map((cover) => cover.text())).toEqual(["Middlemarch", "Circe"]);

    const friendsRow = page.get("a.friends-row");
    expect(friendsRow.attributes("href")).toBe("/friends");
    expect(friendsRow.findAll(".faces .avatar")).toHaveLength(2);
    expect(friendsRow.text()).toContain("1 reading now");
    expect(friendsRow.get(".waiting [aria-hidden=true]").text()).toBe("2");
    expect(friendsRow.get(".waiting .visually-hidden").text()).toBe("2 requests or invitations waiting");

    const clubs = page.findAll(".club-row");
    expect(clubs.map((row) => row.attributes("href"))).toEqual(["/c/readers-ab12", "/c/night-owls-cd34"]);
    expect(clubs[0]!.get(".club-name").text()).toBe("Readers");
    expect(clubs[0]!.text()).toContain("3 members");
    expect(clubs[0]!.get(".cover-stub").text()).toBe("Ulysses");
    expect(clubs[1]!.find(".cover-stub").exists()).toBe(false);
    expect(clubs[1]!.find(".no-book").exists()).toBe(true);
    expect(clubs[1]!.text()).toContain("1 member");
  });

  it("invites instead of empty lists: favourites, friends, clubs", async () => {
    api.myShelf.mockResolvedValue({ user: { id: 1, username: "ada", avatar_url: null }, timezone: "UTC", items: [], favorites: [] });
    api.personalStats.mockResolvedValue({ username: "ada", year: 2026, years: [2026], finished: 0 });
    const page = await mountProfile({ friends: friendsOut() });
    expect(api.clubs).not.toHaveBeenCalled();
    expect(page.findAll(".number-value").map((value) => value.text())).toEqual(["0", "0", "0"]);
    expect(page.get(".portrait-empty").text()).toBe("No favourites yet. Mark up to three from a book page.");
    expect(page.find(".edit-favorites").exists()).toBe(false);
    expect(page.find(".friends-row").exists()).toBe(false);
    const invite = page.get("a.friends-invite");
    expect(invite.attributes("href")).toBe("/friends?add=1");
    expect(invite.text()).toBe("Add friends");
    expect(page.find(".club-row").exists()).toBe(false);
    expect(page.findAll(".clubs-invite a").map((link) => [link.attributes("href"), link.text()])).toEqual([
      ["/clubs/new", "Create a club"],
      ["/clubs", "Join with a code"],
    ]);
  });

  it("keeps a dash where a number could not be loaded, and no friends row before the lists", async () => {
    api.myShelf.mockRejectedValue(new ApiError("offline", 503, "offline"));
    api.personalStats.mockRejectedValue(new ApiError("offline", 503, "offline"));
    const page = await mountProfile();
    expect(page.findAll(".number-value").map((value) => value.text())).toEqual(["–", "–", "–"]);
    expect(page.findAll(".number").map((link) => link.attributes("href"))).toEqual(["/stats", "/shelf", "/friends"]);
    expect(page.find(".friends-row").exists()).toBe(false);
    expect(page.find(".friends-invite").exists()).toBe(false);
  });

  it("changes the picture from the camera badge's sheet", async () => {
    const page = await mountProfile({ avatar: "/media/pictures/abc.webp" });
    await page.get(".photo-badge").trigger("click");
    let sheet = page.get(".sheet");
    expect(sheet.get("h2").text()).toBe("Profile picture");
    expect(sheet.findAll(".sheet-label").map((label) => label.text())).toEqual(["Choose photo", "Remove photo"]);

    const file = new File(["x"], "me.png", { type: "image/png" });
    api.uploadAvatar.mockResolvedValue(user({ avatar_url: "/media/pictures/new.webp" }));
    const input = sheet.get("input[type=file]");
    Object.defineProperty(input.element, "files", { value: [file], configurable: true });
    await input.trigger("change");
    await flushPromises();
    expect(api.uploadAvatar).toHaveBeenCalledWith(file);
    expect(useSession().user?.avatar_url).toBe("/media/pictures/new.webp");
    expect(toasts()).toContain("Profile picture saved");
    expect(page.find(".sheet").exists()).toBe(false);

    await page.get(".photo-badge").trigger("click");
    sheet = page.get(".sheet");
    api.deleteAvatar.mockResolvedValue(user({ avatar_url: null }));
    await sheet.get("button.danger").trigger("click");
    await flushPromises();
    expect(api.deleteAvatar).toHaveBeenCalled();
    expect(toasts()).toContain("Profile picture removed");

    // Without a picture there is nothing to remove.
    await page.get(".photo-badge").trigger("click");
    expect(page.get(".sheet").findAll(".sheet-label").map((label) => label.text())).toEqual(["Choose photo"]);
  });

  it("refuses a picture over 50 MB before sending it", async () => {
    const page = await mountProfile();
    await page.get(".photo-badge").trigger("click");
    const input = page.get(".sheet input[type=file]");
    const big = new File(["x"], "big.png", { type: "image/png" });
    Object.defineProperty(big, "size", { value: 51 * 1024 * 1024 });
    Object.defineProperty(input.element, "files", { value: [big], configurable: true });
    await input.trigger("change");
    await flushPromises();
    expect(api.uploadAvatar).not.toHaveBeenCalled();
    expect(toasts()).toContain(i18n.global.t("errors.pictureTooLarge"));
  });

  it("orders and clears the favourites in a sheet", async () => {
    const page = await mountProfile();
    await page.get(".edit-favorites").trigger("click");
    const sheet = page.get(".sheet");
    expect(sheet.get("h2").text()).toBe("Favourites");

    api.replaceFavorites.mockResolvedValue({ items: [favorite(1, { id: 8, title: "Circe" }), favorite(2, { id: 7, title: "Middlemarch" })] });
    await sheet.get("button[aria-label='Move down']").trigger("click");
    await flushPromises();
    expect(api.replaceFavorites).toHaveBeenCalledWith([8, 7]);
    expect(toasts()).toContain("Favourites updated");
    expect(page.findAll(".portrait .cover-stub").map((cover) => cover.text())).toEqual(["Circe", "Middlemarch"]);

    api.replaceFavorites.mockResolvedValue({ items: [] });
    const clearAll = page.get(".sheet").findAll("button").find((button) => button.text() === "Clear all")!;
    await clearAll.trigger("click");
    await flushPromises();
    expect(api.replaceFavorites).toHaveBeenLastCalledWith([]);
    // Nothing left to order: the sheet goes, and the invitation is back.
    expect(page.find(".sheet").exists()).toBe(false);
    expect(page.find(".portrait-empty").exists()).toBe(true);
  });
});
