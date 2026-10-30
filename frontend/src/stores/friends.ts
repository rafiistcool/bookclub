import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";
import type { FriendsOut } from "../api/generated";
import { useSession } from "./session";

/** A tab that comes back to the front asks for the lists again only when they are older than this. */
export const RELOAD_AFTER_MS = 60_000;

/**
 * A 404 or 409 to a friend action: the other account (or another tab) changed
 * the pair first, e.g. withdrew the request or ended the friendship, so the
 * lists on screen no longer match the server. Only a success answers with
 * the lists, so the store loads them again.
 */
export function isStale(err: unknown): boolean {
  return err instanceof ApiError && (err.status === 404 || err.status === 409);
}

// One `GET /api/friends` at a time per store: the shell and the Friends page both ask on mount.
const inFlight = new WeakMap<object, Promise<void>>();

/**
 * The caller's friends, requests, blocks and personal club invitations
 * (`GET /api/friends`). Every route answers with the whole `FriendsOut`, so
 * each action simply replaces the lists; a refusal because the other side
 * changed the pair first (`isStale`) loads them again. The badge (requests
 * and invitations waiting) is a hint: a failed load keeps what the store had
 * and shows no error; only the Friends page says so, and only when it has
 * nothing to show.
 */
export const useFriends = defineStore("friends", {
  state: () => ({
    data: null as FriendsOut | null,
    /** The account `data` belongs to: another account signing in on this tab never sees it. */
    owner: null as number | null,
    /** The last load failed. */
    failed: false,
    /** When the last load began (ms since the epoch), for the once-a-minute rule. */
    loadedAt: 0,
    /**
     * Raised by every answer the store takes: a load that began before a
     * write's answer arrived must not put the older lists back.
     */
    answers: 0,
  }),
  getters: {
    /** The signed-in account's lists, or null before the first answer. */
    lists(state): FriendsOut | null {
      const id = useSession().user?.id ?? null;
      return state.data && state.owner === id ? state.data : null;
    },
    /** Requests to the caller and club invitations waiting for them. */
    badge(): number {
      const lists = this.lists;
      return lists ? lists.incoming.length + lists.club_invites.length : 0;
    },
  },
  actions: {
    /** Puts an answer in place; `owner` is the account that asked (the signed-in one by default). */
    take(out: FriendsOut, owner = useSession().user?.id ?? null): FriendsOut {
      this.answers += 1;
      this.data = out;
      this.owner = owner;
      this.failed = false;
      return out;
    },
    /** Asks for the lists. A failure leaves them as they were. */
    load(): Promise<void> {
      const running = inFlight.get(this);
      if (running) return running;
      const started = this.answers;
      const owner = useSession().user?.id ?? null;
      this.loadedAt = Date.now();
      // The callbacks run after `set` below, so `delete` always finds this promise.
      const loading = Promise.resolve()
        .then(() => api.friends())
        .then(
          (out) => {
            if (this.answers === started) this.take(out, owner);
          },
          () => {
            this.failed = true;
          },
        )
        .finally(() => inFlight.delete(this));
      inFlight.set(this, loading);
      return loading;
    },
    /** `load()`, unless the last one began less than a minute ago. */
    async loadIfStale(): Promise<void> {
      if (Date.now() - this.loadedAt < RELOAD_AFTER_MS) return;
      await this.load();
    },
    /**
     * `load()` after the one running, if any: that one began before the lists
     * went stale and may not show the change.
     */
    async refresh(): Promise<void> {
      await inFlight.get(this);
      await this.load();
    },
    /** Takes a write's answer; a stale refusal (`isStale`) refreshes the lists, and the error goes on. */
    async write(call: () => Promise<FriendsOut>): Promise<FriendsOut> {
      try {
        return this.take(await call());
      } catch (err) {
        if (isStale(err)) void this.refresh();
        throw err;
      }
    },
    send(username: string) {
      return this.write(() => api.sendFriendRequest(username));
    },
    accept(username: string) {
      return this.write(() => api.acceptFriendRequest(username));
    },
    /** Declines their request, or withdraws one's own. */
    deleteRequest(username: string) {
      return this.write(() => api.deleteFriendRequest(username));
    },
    remove(username: string) {
      return this.write(() => api.removeFriend(username));
    },
    block(username: string) {
      return this.write(() => api.blockAccount(username));
    },
    unblock(username: string) {
      return this.write(() => api.unblockAccount(username));
    },
    declineClubInvite(id: number) {
      return this.write(() => api.declineClubInvite(id));
    },
  },
});

/**
 * Loads the lists again whenever the tab becomes visible, at most once a
 * minute (`AppShell` installs it while it is mounted). Returns the remover.
 */
export function reloadWhenVisible(): () => void {
  const friends = useFriends();
  const onChange = () => {
    if (document.visibilityState === "visible") void friends.loadIfStale();
  };
  document.addEventListener("visibilitychange", onChange);
  return () => document.removeEventListener("visibilitychange", onChange);
}
