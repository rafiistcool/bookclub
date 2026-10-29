import { defineStore } from "pinia";
import { api } from "../api/client";
import type { FriendsOut } from "../api/generated";
import { useSession } from "./session";

/** A tab that comes back to the front asks for the lists again only when they are older than this. */
export const RELOAD_AFTER_MS = 60_000;

// One `GET /api/friends` at a time per store: the shell and the Friends page both ask on mount.
const inFlight = new WeakMap<object, Promise<void>>();

/**
 * The caller's friends, requests, blocks and personal club invitations
 * (`GET /api/friends`). Every route answers with the whole `FriendsOut`, so
 * each action simply replaces the lists. The badge (requests and invitations
 * waiting) is a hint: a failed load keeps what the store had and shows no
 * error; only the Friends page says so, and only when it has nothing to show.
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
    async send(username: string) {
      return this.take(await api.sendFriendRequest(username));
    },
    async accept(username: string) {
      return this.take(await api.acceptFriendRequest(username));
    },
    /** Declines their request, or withdraws one's own. */
    async deleteRequest(username: string) {
      return this.take(await api.deleteFriendRequest(username));
    },
    async remove(username: string) {
      return this.take(await api.removeFriend(username));
    },
    async block(username: string) {
      return this.take(await api.blockAccount(username));
    },
    async unblock(username: string) {
      return this.take(await api.unblockAccount(username));
    },
    async declineClubInvite(id: number) {
      return this.take(await api.declineClubInvite(id));
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
