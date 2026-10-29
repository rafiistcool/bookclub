import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { ClubSummary } from "../types";
import { user } from "../test/fixtures";
import { blockedOut, friendClubInvite, friendOut, friendRequest, friendsOut, reading } from "../test/friends";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  friends: vi.fn(),
  sendFriendRequest: vi.fn(),
  acceptFriendRequest: vi.fn(),
  deleteFriendRequest: vi.fn(),
  removeFriend: vi.fn(),
  blockAccount: vi.fn(),
  unblockAccount: vi.fn(),
  declineClubInvite: vi.fn(),
  createInvite: vi.fn(),
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
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { translateApiError } from "../i18n/apiErrors";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import FriendsPage from "./FriendsPage.vue";

const READERS: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "admin", timezone: "UTC", member_count: 3 };

/** The server's error as the client throws it: translated message, English detail. */
function apiError(status: number, detail: string) {
  return new ApiError(translateApiError(detail), status, detail);
}

const FULL = friendsOut({
  friends: [
    friendOut("grace", {
      since: "2026-03-12T10:00:00Z",
      currently_reading_count: 5,
      currently_reading: [reading(11, 40, "Circe"), reading(12, null, "Middlemarch"), reading(13, 5, "Ulysses")],
    }),
    friendOut("hal"),
  ],
  incoming: [friendRequest("mia", { created_at: "2026-10-20T09:00:00Z" })],
  outgoing: [friendRequest("olga")],
  blocked: [blockedOut("max")],
  club_invites: [friendClubInvite({ id: 4, invited_by: "grace", club: { slug: "night-owls-cd34", name: "Night owls" } })],
});

let wrapper: VueWrapper | null = null;

async function mountPage(lists = FULL, clubs: ClubSummary[] = [READERS]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ id: 1, username: "ada", clubs, timezone: "UTC" });
  api.friends.mockResolvedValue(lists);
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/friends", component: FriendsPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push("/friends");
  wrapper = mount(FriendsPage, {
    attachTo: document.body,
    global: { plugins: [pinia, router, i18n], stubs: { BookCover: true } },
  });
  await flushPromises();
  return wrapper;
}

function section(page: VueWrapper, id: string) {
  return page.get(`section[aria-labelledby=${id}]`);
}

function button(scope: { findAll: VueWrapper["findAll"] }, label: string) {
  const found = scope.findAll("button").find((element) => element.text() === label);
  if (!found) throw new Error(`no button "${label}"`);
  return found;
}

function toasts(): string[] {
  return useToast().items.map((toast) => toast.message);
}

describe("FriendsPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    document.body.innerHTML = "";
  });

  it("shows the requests, invitations, friends, sent requests and blocks, in that order", async () => {
    const page = await mountPage();
    expect(api.friends).toHaveBeenCalledTimes(1);
    expect(page.findAll("h2").map((heading) => heading.text())).toEqual([
      "Add a friend",
      "Requests to you",
      "Invitations to clubs",
      "Your friends",
      "Requests you sent",
      "Blocked",
    ]);
    expect(page.get(".add-friend").text()).toContain("Type their exact username.");

    const incoming = section(page, "friend-requests").get(".incoming-row");
    expect(incoming.text()).toContain("mia");
    expect(incoming.text()).toContain("asked 20 Oct 2026");

    const invite = section(page, "club-invitations").get(".club-invite-row");
    expect(invite.text()).toContain("grace invited you to Night owls");
    expect(invite.get("a").attributes("href")).toBe("/join/7K3M9QX2HD");

    const cards = section(page, "your-friends").findAll(".friend-card");
    expect(cards.map((card) => card.get(".person-meta strong").text())).toEqual(["grace", "hal"]);
    const grace = cards[0]!;
    expect(grace.get(".friend-link").attributes("href")).toBe("/u/grace");
    expect(grace.text()).toContain("Friends since 12 Mar 2026");
    expect(grace.findAll(".friend-book-title").map((title) => title.text())).toEqual(["Circe", "Middlemarch", "Ulysses"]);
    expect(grace.findAll(".friend-book").map((row) => row.attributes("href"))).toEqual(["/book/11", "/book/12", "/book/13"]);
    expect(grace.text()).toContain("40%");
    expect(grace.get(".more-link").text()).toBe("+2 more");
    expect(grace.get(".more-link").attributes("href")).toBe("/u/grace");
    expect(cards[1]!.text()).toContain("Not reading anything right now");
    expect(cards[1]!.find(".more-link").exists()).toBe(false);

    expect(section(page, "sent-requests").get(".outgoing-row").text()).toContain("olga");
    const blocked = section(page, "blocked-accounts");
    expect(blocked.get(".blocked-row").text()).toContain("max");
    // Answer 21: a block does not reach into shared clubs, and the page says so.
    expect(blocked.get(".block-note").text()).toBe(
      "Blocking ends the friendship and hides their requests. It does not hide anything in clubs you share now or join later. To stop that, leave the club or ask its admin to remove them.",
    );
  });

  it("has an empty state for every list", async () => {
    const page = await mountPage(friendsOut());
    expect(page.findAll(".empty-line").map((line) => line.text())).toEqual([
      "No requests right now.",
      "No invitations right now.",
      "No friends yet. Send a request to someone's exact username above.",
      "No open requests.",
      "You have not blocked anyone.",
    ]);
  });

  it("sends a request by exact username and says what became of it", async () => {
    const page = await mountPage(friendsOut());
    const input = page.get(".add-form input");
    const submit = page.get(".add-form button[type=submit]");
    expect(submit.attributes("disabled")).toBeDefined();

    api.sendFriendRequest.mockResolvedValue(friendsOut({ outgoing: [friendRequest("grace")] }));
    await input.setValue(" Grace ");
    await page.get(".add-form").trigger("submit");
    await flushPromises();
    expect(api.sendFriendRequest).toHaveBeenCalledWith("Grace");
    expect(toasts()).toContain("Request sent to grace");
    expect((input.element as HTMLInputElement).value).toBe("");
    expect(section(page, "sent-requests").text()).toContain("grace");

    // Their request was already waiting: the two are friends now.
    api.sendFriendRequest.mockResolvedValue(friendsOut({ friends: [friendOut("mia")] }));
    await input.setValue("mia");
    await page.get(".add-form").trigger("submit");
    await flushPromises();
    expect(toasts()).toContain("You and mia are friends now");

    // A refusal stays by the field, with the name, for a correction.
    api.sendFriendRequest.mockRejectedValue(apiError(404, "No account with that name"));
    await input.setValue("nobody");
    await page.get(".add-form").trigger("submit");
    await flushPromises();
    expect(page.get(".add-form .error").text()).toBe("No account with that name");
    expect((input.element as HTMLInputElement).value).toBe("nobody");

    api.sendFriendRequest.mockRejectedValue(apiError(409, "You blocked this account"));
    await page.get(".add-form").trigger("submit");
    await flushPromises();
    expect(page.get(".add-form .error").text()).toBe("You blocked this account. Unblock it first.");

    api.sendFriendRequest.mockRejectedValue(apiError(400, "That is your own account"));
    await input.setValue("ada");
    await page.get(".add-form").trigger("submit");
    await flushPromises();
    expect(page.get(".add-form .error").text()).toBe("That is your own account");
  });

  it("accepts and declines requests, and blocks only after a confirmation", async () => {
    const page = await mountPage();
    const incoming = () => section(page, "friend-requests");

    api.acceptFriendRequest.mockResolvedValue(friendsOut({ friends: [friendOut("mia")] }));
    await button(incoming(), "Accept").trigger("click");
    await flushPromises();
    expect(api.acceptFriendRequest).toHaveBeenCalledWith("mia");
    expect(toasts()).toContain("You and mia are friends now");
    expect(incoming().text()).toContain("No requests right now.");
    expect(section(page, "your-friends").text()).toContain("mia");

    await wrapper!.unmount();
    const again = await mountPage();
    api.deleteFriendRequest.mockResolvedValue(friendsOut());
    await button(section(again, "friend-requests"), "Decline").trigger("click");
    await flushPromises();
    expect(api.deleteFriendRequest).toHaveBeenCalledWith("mia");
    expect(toasts()).toContain("Request declined");

    await wrapper!.unmount();
    const third = await mountPage();
    await button(section(third, "friend-requests"), "Block").trigger("click");
    const sheet = third.get(".sheet");
    expect(sheet.get("h2").text()).toBe("Block mia?");
    expect(sheet.text()).toContain("mia is not told.");
    expect(sheet.text()).toContain("It does not hide anything in clubs you share: there you still see each other.");
    expect(api.blockAccount).not.toHaveBeenCalled();
    api.blockAccount.mockResolvedValue(friendsOut({ blocked: [blockedOut("mia")] }));
    await button(sheet, "Yes, block").trigger("click");
    await flushPromises();
    expect(api.blockAccount).toHaveBeenCalledWith("mia");
    expect(toasts()).toContain("mia is blocked");
    expect(third.find(".sheet").exists()).toBe(false);
    expect(section(third, "blocked-accounts").get(".blocked-row").text()).toContain("mia");
  });

  it("offers each friend an invitation into a club, removal and a block, each confirmed", async () => {
    const page = await mountPage();
    const grace = () => section(page, "your-friends").findAll(".friend-card")[0]!;
    const options = grace().get(".options-btn");
    expect(options.attributes("aria-label")).toBe("Options for grace");

    await options.trigger("click");
    let sheet = page.get(".sheet");
    expect(sheet.get("h2").text()).toBe("grace");
    await button(sheet, "Invite to a club…").trigger("click");
    sheet = page.get(".sheet");
    expect(sheet.get("h2").text()).toBe("Invite grace to a club");
    expect(sheet.text()).toContain("Readers");
    await button(sheet, "Done").trigger("click");
    expect(page.find(".sheet").exists()).toBe(false);

    await grace().get(".options-btn").trigger("click");
    await button(page.get(".sheet"), "Remove from friends").trigger("click");
    sheet = page.get(".sheet");
    expect(sheet.get("h2").text()).toBe("Remove grace from your friends?");
    expect(api.removeFriend).not.toHaveBeenCalled();
    api.removeFriend.mockResolvedValue(friendsOut({ friends: [friendOut("hal")] }));
    await button(sheet, "Yes, remove").trigger("click");
    await flushPromises();
    expect(api.removeFriend).toHaveBeenCalledWith("grace");
    expect(toasts()).toContain("You and grace are no longer friends");
    expect(section(page, "your-friends").findAll(".friend-card")).toHaveLength(1);

    const hal = section(page, "your-friends").get(".friend-card");
    await hal.get(".options-btn").trigger("click");
    await button(page.get(".sheet"), "Block").trigger("click");
    expect(page.get(".sheet h2").text()).toBe("Block hal?");
    api.blockAccount.mockRejectedValue(apiError(429, "Slow down a little. Try again later."));
    await button(page.get(".sheet"), "Yes, block").trigger("click");
    await flushPromises();
    // A failure keeps the sheet open, with the reason in a toast.
    expect(page.find(".sheet").exists()).toBe(true);
    expect(toasts()).toContain(translateApiError("Slow down a little. Try again later."));
  });

  it("does not offer an invitation to a reader who runs no club", async () => {
    const page = await mountPage(FULL, [{ ...READERS, role: "member" }]);
    await section(page, "your-friends").get(".options-btn").trigger("click");
    const labels = page.get(".sheet").findAll("button").map((element) => element.text());
    expect(labels).toEqual(["Remove from friends", "Block", "Cancel"]);
  });

  it("withdraws sent requests, unblocks, and declines club invitations", async () => {
    const page = await mountPage();
    api.deleteFriendRequest.mockResolvedValue({ ...FULL, outgoing: [] });
    await button(section(page, "sent-requests"), "Withdraw").trigger("click");
    await flushPromises();
    expect(api.deleteFriendRequest).toHaveBeenCalledWith("olga");
    expect(toasts()).toContain("Request withdrawn");

    api.unblockAccount.mockResolvedValue({ ...FULL, outgoing: [], blocked: [] });
    await button(section(page, "blocked-accounts"), "Unblock").trigger("click");
    await flushPromises();
    expect(api.unblockAccount).toHaveBeenCalledWith("max");
    expect(toasts()).toContain("max is no longer blocked");

    api.declineClubInvite.mockResolvedValue(friendsOut());
    await button(section(page, "club-invitations"), "Decline").trigger("click");
    await flushPromises();
    expect(api.declineClubInvite).toHaveBeenCalledWith(4);
    expect(toasts()).toContain("Invitation declined");
    expect(section(page, "club-invitations").text()).toContain("No invitations right now.");
  });

  it("says when the lists cannot be loaded, and tries again", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ id: 1 });
    api.friends.mockRejectedValue(new Error("offline"));
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:rest(.*)*", component: FriendsPage }] });
    wrapper = mount(FriendsPage, { global: { plugins: [pinia, router, i18n], stubs: { BookCover: true } } });
    await flushPromises();
    expect(wrapper.get(".load-failed .error").text()).toBe("Could not load your friends");
    // Sending still works without the lists.
    expect(wrapper.find(".add-form").exists()).toBe(true);

    api.friends.mockResolvedValue(friendsOut({ incoming: [friendRequest("mia")] }));
    await button(wrapper.get(".load-failed"), "Try again").trigger("click");
    await flushPromises();
    expect(wrapper.find(".load-failed").exists()).toBe(false);
    expect(wrapper.get(".incoming-row").text()).toContain("mia");
  });
});
