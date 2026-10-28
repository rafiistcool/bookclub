import { defineStore } from "pinia";
import { computed, type ComputedRef } from "vue";
import { api, ApiError } from "../api/client";
import type { ClubOut, ClubSummary } from "../types";
import { useSession } from "./session";

/** The club this device opened last (a per-device convenience, like the theme). */
export const LAST_CLUB_KEY = "bookclub.club";

/** The detail every `/api/clubs/{slug}/…` route answers a non-member or an unknown slug. */
export const NO_SUCH_CLUB = "No such club";

export type ClubRole = "owner" | "admin" | "member";

function readLastClub(): string | null {
  try {
    return localStorage.getItem(LAST_CLUB_KEY) || null;
  } catch {
    return null;
  }
}

function writeLastClub(slug: string | null) {
  try {
    if (slug) localStorage.setItem(LAST_CLUB_KEY, slug);
    else localStorage.removeItem(LAST_CLUB_KEY);
  } catch {
    /* Private mode or blocked storage: the club is simply not remembered. */
  }
}

/** Admins and the owner run the club: picks, the vote's end, invites, milestones, removals. */
export function isClubAdmin(role: string | null | undefined): boolean {
  return role === "owner" || role === "admin";
}

export function isClubOwner(role: string | null | undefined): boolean {
  return role === "owner";
}

/** True when a club request says the viewer is not (or no longer) in that club. */
export function isClubGone(err: unknown): boolean {
  return err instanceof ApiError && err.status === 404 && err.detail === NO_SUCH_CLUB;
}

export function clubSummary(club: ClubOut): ClubSummary {
  return {
    slug: club.slug,
    name: club.name,
    role: club.role,
    timezone: club.timezone,
    member_count: club.member_count,
  };
}

/** The club "the club" means: the last-used one while the viewer is still in it, else the only one. */
export function defaultClubSlug(
  clubs: readonly { slug: string }[],
  lastSlug: string | null,
): string | null {
  if (lastSlug && clubs.some((club) => club.slug === lastSlug)) return lastSlug;
  if (clubs.length === 1) return clubs[0]!.slug;
  return null;
}

/**
 * Where a club link without a slug leads (`/club`, the Club tab, old routes):
 * `sub` of the default club, else the club list, which is also the
 * create-or-join screen of a reader with no club.
 */
export function clubLanding(
  clubs: readonly { slug: string }[],
  lastSlug: string | null,
  sub = "",
): string {
  const slug = defaultClubSlug(clubs, lastSlug);
  return slug ? `/c/${slug}${sub}` : "/clubs";
}

/** `/c/<slug>/rest` with another club's slug, so the switcher keeps the section. */
export function switchClubPath(path: string, slug: string): string {
  const match = /^\/c\/[^/]+(\/.*)?$/.exec(path);
  return `/c/${slug}${match?.[1] ?? ""}`;
}

/**
 * The viewer's clubs. The switcher's list is `UserOut.clubs` (it comes with
 * the session, so it costs no request); `GET /api/clubs` adds each club's
 * current pick for the cards and is only asked when the viewer has a club.
 */
export const useClubs = defineStore("clubs", {
  state: () => ({
    lastSlug: readLastClub(),
    /** `GET /api/clubs`, newest join first. */
    list: [] as ClubOut[],
    listLoaded: false,
    /** The club whose pages are open (`GET /api/clubs/{slug}`). */
    current: null as ClubOut | null,
    /** A slug that answered 404: the club page shows "club not found". */
    goneSlug: null as string | null,
  }),
  getters: {
    items(): ClubSummary[] {
      return useSession().user?.clubs ?? [];
    },
    hasClubs(): boolean {
      return this.items.length > 0;
    },
    defaultSlug(): string | null {
      return defaultClubSlug(this.items, this.lastSlug);
    },
  },
  actions: {
    remember(slug: string) {
      this.lastSlug = slug;
      writeLastClub(slug);
    },
    /** The viewer is not in this club (any more): forget it here, in storage and in the switcher. */
    forget(slug: string) {
      if (this.lastSlug === slug) {
        this.lastSlug = null;
        writeLastClub(null);
      }
      const session = useSession();
      if (session.user) session.user.clubs = session.user.clubs.filter((club) => club.slug !== slug);
      this.list = this.list.filter((club) => club.slug !== slug);
      if (this.current?.slug === slug) this.current = null;
    },
    /** A club request answered 404 `No such club`: show "not found" and drop the slug. */
    markGone(slug: string) {
      this.forget(slug);
      this.goneSlug = slug;
    },
    /** Keep the switcher and the cards in step with a club the server just answered with. */
    upsert(club: ClubOut) {
      const session = useSession();
      if (session.user) {
        const rest = session.user.clubs.filter((row) => row.slug !== club.slug);
        const at = session.user.clubs.findIndex((row) => row.slug === club.slug);
        const summary = clubSummary(club);
        session.user.clubs = at < 0 ? [summary, ...rest] : session.user.clubs.map((row) => (row.slug === club.slug ? summary : row));
      }
      const at = this.list.findIndex((row) => row.slug === club.slug);
      if (at < 0) this.list = [club, ...this.list];
      else this.list = this.list.map((row) => (row.slug === club.slug ? club : row));
      if (this.current?.slug === club.slug) this.current = club;
      if (this.goneSlug === club.slug) this.goneSlug = null;
    },
    /** `GET /api/clubs`. Solo mode makes no club call: no clubs, no request. */
    async loadList(): Promise<ClubOut[]> {
      if (!this.hasClubs) {
        this.list = [];
        this.listLoaded = true;
        return this.list;
      }
      const { items } = await api.clubs();
      this.list = items;
      this.listLoaded = true;
      const session = useSession();
      if (session.user) session.user.clubs = items.map(clubSummary);
      return items;
    },
    /** After a join, create, leave, transfer or delete: ask the server which clubs the viewer is in. */
    async refresh() {
      const session = useSession();
      if (!session.user) return;
      const { items } = await api.clubs();
      this.list = items;
      this.listLoaded = true;
      session.user.clubs = items.map(clubSummary);
      if (this.lastSlug && !items.some((club) => club.slug === this.lastSlug)) {
        this.lastSlug = null;
        writeLastClub(null);
      }
    },
    /** Another account signs in on this tab: nothing of the previous one's clubs stays. */
    clear() {
      this.list = [];
      this.listLoaded = false;
      this.current = null;
      this.goneSlug = null;
      this.lastSlug = readLastClub();
    },
  },
});

/**
 * The club of the open `/c/:slug/…` section. The club layout renders a
 * section only once the club has answered and keys it by slug, so this is
 * never empty there; after a leave or delete it keeps the last answer until
 * the section is gone.
 */
export function useCurrentClub(): ComputedRef<ClubOut> {
  const clubs = useClubs();
  const initial = clubs.current!;
  return computed(() => (clubs.current?.slug === initial.slug ? clubs.current : initial));
}
