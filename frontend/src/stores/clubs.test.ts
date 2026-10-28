import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, clubOut, clubPick, user } from "../test/fixtures";
import type { ClubRole, ClubSummary } from "../types";
import { blockedStorage, memoryStorage } from "../test/storage";

const clubsList = vi.fn();

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
  return { ApiError, api: { clubs: (...args: unknown[]) => clubsList(...args) } };
});

import { ApiError } from "../api/client";
import {
  clubLanding,
  defaultClubSlug,
  isClubGone,
  LAST_CLUB_KEY,
  switchClubPath,
  useClubs,
} from "./clubs";
import { useSession } from "./session";

const A = { slug: "readers-ab12", name: "Readers" };
const B = { slug: "night-owls-cd34", name: "Night owls" };

function summary(club: { slug: string; name: string }, role: ClubRole = "member"): ClubSummary {
  return { ...club, role, timezone: "Europe/Berlin", member_count: 3 };
}

describe("where a club link without a slug leads", () => {
  it("prefers the last-used club while the viewer is still in it", () => {
    expect(defaultClubSlug([A, B], B.slug)).toBe(B.slug);
    expect(clubLanding([A, B], B.slug, "/overlap")).toBe(`/c/${B.slug}/overlap`);
  });

  it("falls back to the only club, then to the club list", () => {
    expect(clubLanding([A], "gone-zz99")).toBe(`/c/${A.slug}`);
    expect(clubLanding([A, B], "gone-zz99")).toBe("/clubs");
    expect(clubLanding([A, B], null, "/members")).toBe("/clubs");
    // Solo mode: the club list is the create-or-join screen.
    expect(clubLanding([], A.slug)).toBe("/clubs");
  });

  it("keeps the section when the switcher changes the club", () => {
    expect(switchClubPath(`/c/${A.slug}/vote`, B.slug)).toBe(`/c/${B.slug}/vote`);
    expect(switchClubPath(`/c/${A.slug}`, B.slug)).toBe(`/c/${B.slug}`);
    expect(switchClubPath("/clubs", B.slug)).toBe(`/c/${B.slug}`);
  });

  it("tells a club 404 from any other error", () => {
    expect(isClubGone(new ApiError("x", 404, "No such club"))).toBe(true);
    expect(isClubGone(new ApiError("x", 404, "No club pick yet"))).toBe(false);
    expect(isClubGone(new ApiError("x", 403, "No such club"))).toBe(false);
    expect(isClubGone(new Error("offline"))).toBe(false);
  });
});

describe("useClubs", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clubsList.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("remembers the last-used club on this device", () => {
    useClubs().remember(A.slug);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe(A.slug);
    setActivePinia(createPinia());
    expect(useClubs().lastSlug).toBe(A.slug);
  });

  it("works without storage", () => {
    vi.stubGlobal("localStorage", blockedStorage());
    const clubs = useClubs();
    expect(clubs.lastSlug).toBeNull();
    expect(() => clubs.remember(A.slug)).not.toThrow();
    expect(clubs.lastSlug).toBe(A.slug);
    expect(() => clubs.forget(A.slug)).not.toThrow();
  });

  it("drops a club that answered 404 from storage and the switcher", () => {
    localStorage.setItem(LAST_CLUB_KEY, A.slug);
    const session = useSession();
    session.user = user({ clubs: [summary(A), summary(B)] });
    const clubs = useClubs();
    expect(clubs.defaultSlug).toBe(A.slug);

    clubs.markGone(A.slug);

    expect(clubs.goneSlug).toBe(A.slug);
    expect(clubs.lastSlug).toBeNull();
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBeNull();
    expect(session.user.clubs.map((club) => club.slug)).toEqual([B.slug]);
    expect(clubs.defaultSlug).toBe(B.slug);
  });

  it("keeps another club's last-used slug when one club goes", () => {
    localStorage.setItem(LAST_CLUB_KEY, B.slug);
    useSession().user = user({ clubs: [summary(A), summary(B)] });
    const clubs = useClubs();
    clubs.forget(A.slug);
    expect(clubs.lastSlug).toBe(B.slug);
  });

  it("makes no club call in solo mode", async () => {
    useSession().user = user({ clubs: [] });
    const clubs = useClubs();
    expect(await clubs.loadList()).toEqual([]);
    expect(clubsList).not.toHaveBeenCalled();
  });

  it("loads the cards and keeps the switcher in step", async () => {
    const session = useSession();
    session.user = user({ clubs: [summary(A)] });
    clubsList.mockResolvedValue({ items: [clubOut({ ...A, name: "Readers (renamed)", role: "admin" })] });
    const clubs = useClubs();
    await clubs.loadList();
    expect(clubs.list.map((club) => club.name)).toEqual(["Readers (renamed)"]);
    expect(session.user.clubs[0]).toMatchObject({ slug: A.slug, name: "Readers (renamed)", role: "admin" });
  });

  it("adds a joined club in front and updates a known one in place", () => {
    const session = useSession();
    session.user = user({ clubs: [summary(A)] });
    const clubs = useClubs();
    clubs.upsert(clubOut({ ...B, role: "member" }));
    expect(session.user.clubs.map((club) => club.slug)).toEqual([B.slug, A.slug]);
    clubs.upsert(clubOut({ ...A, role: "owner", member_count: 4 }));
    expect(session.user.clubs.map((club) => club.slug)).toEqual([B.slug, A.slug]);
    expect(session.user.clubs[1]).toMatchObject({ role: "owner", member_count: 4 });
  });

  it("keeps a club's current pick in step with a pick answer", () => {
    useSession().user = user({ clubs: [summary(A), summary(B)] });
    const clubs = useClubs();
    clubs.current = clubOut({ ...A, current_pick: null });
    clubs.list = [clubOut({ ...A, current_pick: null }), clubOut({ ...B, current_pick: null })];

    clubs.syncPick(A.slug, clubPick({ id: 7, book: bookOut({ id: 3 }), meeting_at: null }));
    expect(clubs.current?.current_pick).toEqual({ id: 7, book: bookOut({ id: 3 }), meeting_at: null });
    expect(clubs.list.map((club) => club.current_pick?.id ?? null)).toEqual([7, null]);

    // An answer that changes nothing leaves the objects alone (no re-render, no reload).
    const before = clubs.current;
    const list = clubs.list;
    clubs.syncPick(A.slug, clubPick({ id: 7, book: bookOut({ id: 3 }), meeting_at: null }));
    expect(clubs.current).toBe(before);
    expect(clubs.list).toBe(list);

    // Another club's answer does not touch the open club.
    clubs.syncPick(B.slug, clubPick({ id: 8 }));
    expect(clubs.current?.current_pick?.id).toBe(7);
    expect(clubs.list[1]?.current_pick?.id).toBe(8);

    clubs.syncPick(A.slug, null);
    expect(clubs.current?.current_pick).toBeNull();
  });

  it("forgets a stale last-used club on refresh", async () => {
    localStorage.setItem(LAST_CLUB_KEY, A.slug);
    useSession().user = user({ clubs: [summary(A)] });
    clubsList.mockResolvedValue({ items: [clubOut(B)] });
    const clubs = useClubs();
    await clubs.refresh();
    expect(clubs.lastSlug).toBeNull();
    expect(useSession().user?.clubs.map((club) => club.slug)).toEqual([B.slug]);
  });
});
