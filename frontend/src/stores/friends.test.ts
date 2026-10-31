import { flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { user } from "../test/fixtures";
import { blockedOut, friendClubInvite, friendOut, friendRequest, friendsOut } from "../test/friends";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  friends: vi.fn(),
  sendFriendRequest: vi.fn(),
  acceptFriendRequest: vi.fn(),
  deleteFriendRequest: vi.fn(),
  removeFriend: vi.fn(),
  blockAccount: vi.fn(),
  unblockAccount: vi.fn(),
  declineClubInvite: vi.fn(),
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
import { RELOAD_AFTER_MS, reloadWhenVisible, useFriends } from "./friends";
import { useSession } from "./session";

/** A promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("the friends store", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    setActivePinia(createPinia());
    useSession().user = user({ id: 1, username: "ada" });
  });

  afterEach(() => {
    vi.useRealTimers();
    Reflect.deleteProperty(document, "visibilityState");
  });

  it("counts requests to the caller and club invitations as the badge, not sent requests or friends", async () => {
    const friends = useFriends();
    expect(friends.badge).toBe(0);
    expect(friends.lists).toBeNull();
    api.friends.mockResolvedValue(
      friendsOut({
        friends: [friendOut("grace")],
        incoming: [friendRequest("mia"), friendRequest("max")],
        outgoing: [friendRequest("olga")],
        club_invites: [friendClubInvite()],
      }),
    );
    await friends.load();
    expect(api.friends).toHaveBeenCalledTimes(1);
    expect(friends.badge).toBe(3);
    expect(friends.lists?.friends.map((row) => row.user.username)).toEqual(["grace"]);
  });

  it("replaces the lists with every action's answer", async () => {
    const friends = useFriends();
    api.friends.mockResolvedValue(friendsOut({ incoming: [friendRequest("grace")] }));
    await friends.load();

    const accepted = friendsOut({ friends: [friendOut("grace")] });
    api.acceptFriendRequest.mockResolvedValue(accepted);
    await expect(friends.accept("grace")).resolves.toBe(accepted);
    expect(api.acceptFriendRequest).toHaveBeenCalledWith("grace");
    expect(friends.lists).toEqual(accepted);
    expect(friends.badge).toBe(0);

    const cases: [() => Promise<unknown>, ReturnType<typeof vi.fn>, unknown][] = [
      [() => friends.send("mia"), api.sendFriendRequest!, "mia"],
      [() => friends.deleteRequest("mia"), api.deleteFriendRequest!, "mia"],
      [() => friends.remove("grace"), api.removeFriend!, "grace"],
      [() => friends.block("max"), api.blockAccount!, "max"],
      [() => friends.unblock("max"), api.unblockAccount!, "max"],
      [() => friends.declineClubInvite(4), api.declineClubInvite!, 4],
    ];
    for (const [run, call, argument] of cases) {
      const answer = friendsOut({ outgoing: [friendRequest(`after-${String(argument)}`)] });
      call.mockResolvedValue(answer);
      await run();
      expect(call).toHaveBeenCalledWith(argument);
      expect(friends.lists).toEqual(answer);
    }
  });

  it("loads the lists again when a write finds the pair changed (404 or 409), and passes the error on", async () => {
    const friends = useFriends();
    api.friends.mockResolvedValue(friendsOut({ incoming: [friendRequest("grace")] }));
    await friends.load();
    expect(friends.badge).toBe(1);

    // Grace withdrew her request before the tap.
    api.friends.mockResolvedValue(friendsOut());
    const gone = new ApiError("That request no longer exists", 404, "No such friend request");
    api.acceptFriendRequest.mockRejectedValue(gone);
    await expect(friends.accept("grace")).rejects.toBe(gone);
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(2);
    expect(friends.badge).toBe(0);

    // Mia was blocked from another tab.
    api.friends.mockResolvedValue(friendsOut({ blocked: [blockedOut("mia")] }));
    api.sendFriendRequest.mockRejectedValue(new ApiError("You blocked this account.", 409, "You blocked this account"));
    await expect(friends.send("mia")).rejects.toBeInstanceOf(ApiError);
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(3);
    expect(friends.lists?.blocked.map((row) => row.username)).toEqual(["mia"]);

    // Any other failure leaves the lists alone.
    api.removeFriend.mockRejectedValue(new ApiError("Slow down", 429, "Slow down a little. Try again later."));
    await expect(friends.remove("hal")).rejects.toBeInstanceOf(ApiError);
    api.blockAccount.mockRejectedValue(new Error("offline"));
    await expect(friends.block("hal")).rejects.toThrow("offline");
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(3);
  });

  it("waits for a load already running before it loads the lists again", async () => {
    const friends = useFriends();
    const slow = deferred<ReturnType<typeof friendsOut>>();
    api.friends.mockReturnValueOnce(slow.promise);
    const running = friends.load();

    api.friends.mockResolvedValueOnce(friendsOut());
    api.deleteFriendRequest.mockRejectedValue(new ApiError("That request no longer exists", 404, "No such friend request"));
    await expect(friends.deleteRequest("grace")).rejects.toBeInstanceOf(ApiError);
    // The running load began before the refusal and may predate it: a second load follows.
    slow.resolve(friendsOut({ incoming: [friendRequest("grace")] }));
    await running;
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(2);
    expect(friends.badge).toBe(0);
  });

  it("keeps the lists and shows no error when a load fails", async () => {
    const friends = useFriends();
    api.friends.mockResolvedValue(friendsOut({ incoming: [friendRequest("grace")] }));
    await friends.load();
    api.friends.mockRejectedValue(new Error("offline"));
    await expect(friends.load()).resolves.toBeUndefined();
    expect(friends.failed).toBe(true);
    expect(friends.badge).toBe(1);
    // The next answer clears the flag.
    api.friends.mockResolvedValue(friendsOut());
    await friends.load();
    expect(friends.failed).toBe(false);
    expect(friends.badge).toBe(0);
  });

  it("asks once while a load is running, and never puts older lists over a write's answer", async () => {
    const friends = useFriends();
    const slow = deferred<ReturnType<typeof friendsOut>>();
    api.friends.mockReturnValue(slow.promise);
    const first = friends.load();
    const second = friends.load();

    const written = friendsOut({ friends: [friendOut("grace")] });
    api.sendFriendRequest.mockResolvedValue(written);
    await friends.send("grace");
    slow.resolve(friendsOut({ outgoing: [friendRequest("grace")] }));
    await Promise.all([first, second]);
    expect(api.friends).toHaveBeenCalledTimes(1);
    expect(friends.lists).toEqual(written);
  });

  it("never shows one account's lists to the next account on the same tab", async () => {
    const friends = useFriends();
    api.friends.mockResolvedValue(friendsOut({ incoming: [friendRequest("grace")] }));
    await friends.load();
    expect(friends.badge).toBe(1);
    useSession().user = user({ id: 2, username: "bob" });
    expect(friends.lists).toBeNull();
    expect(friends.badge).toBe(0);
  });

  it("loads again when the tab comes back, at most once a minute", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-29T12:00:00Z"));
    const friends = useFriends();
    api.friends.mockResolvedValue(friendsOut());
    const stop = reloadWhenVisible();
    await friends.load();
    expect(api.friends).toHaveBeenCalledTimes(1);

    // Within the minute: nothing.
    vi.setSystemTime(new Date(Date.now() + RELOAD_AFTER_MS - 1000));
    setVisibility("visible");
    await vi.waitFor(() => expect(api.friends).toHaveBeenCalledTimes(1));

    // Hidden is not a return.
    vi.setSystemTime(new Date(Date.now() + 2000));
    setVisibility("hidden");
    expect(api.friends).toHaveBeenCalledTimes(1);

    api.friends.mockResolvedValue(friendsOut({ club_invites: [friendClubInvite()] }));
    setVisibility("visible");
    await vi.waitFor(() => expect(friends.badge).toBe(1));
    expect(api.friends).toHaveBeenCalledTimes(2);

    // Removed with the shell: no more loads.
    stop();
    vi.setSystemTime(new Date(Date.now() + 5 * RELOAD_AFTER_MS));
    setVisibility("visible");
    expect(api.friends).toHaveBeenCalledTimes(2);
  });
});
