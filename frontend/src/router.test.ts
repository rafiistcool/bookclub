import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { user } from "./test/fixtures";
import { memoryStorage } from "./test/storage";

const me = vi.fn();

vi.mock("./api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { me: (...args: unknown[]) => me(...args) },
  setUnauthorizedHandler: () => undefined,
}));

import { ApiError } from "./api/client";
import router from "./router";
import { LAST_CLUB_KEY } from "./stores/clubs";

const READERS = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };
const OWLS = { slug: "night-owls-cd34", name: "Night owls", role: "admin", timezone: "UTC", member_count: 2 };

async function landing(path: string, clubs: (typeof READERS)[], lastSlug?: string, isAdmin = false) {
  vi.stubGlobal("localStorage", memoryStorage());
  if (lastSlug) localStorage.setItem(LAST_CLUB_KEY, lastSlug);
  setActivePinia(createPinia());
  me.mockResolvedValue(user({ clubs, is_admin: isAdmin }));
  // Somewhere neutral first: the guard loads the session (and the clubs) once.
  await router.push("/settings");
  const failure = await router.push(path).catch((error: unknown) => error);
  expect(failure).toBeUndefined();
  return router.currentRoute.value.fullPath;
}

describe("club routes without a slug", () => {
  beforeEach(() => {
    me.mockReset();
    // The theme store asks for the colour scheme when the session arrives.
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("leads /club to the last-used club, the only club, or the club list", async () => {
    expect(await landing("/club", [READERS, OWLS], OWLS.slug)).toBe("/c/night-owls-cd34");
    expect(await landing("/club", [READERS])).toBe("/c/readers-ab12");
    expect(await landing("/club", [READERS, OWLS])).toBe("/clubs");
    expect(await landing("/club", [])).toBe("/clubs");
  });

  it("sends the old single-club paths into the club context", async () => {
    expect(await landing("/overlap", [READERS])).toBe("/c/readers-ab12/overlap");
    expect(await landing("/invites", [READERS])).toBe("/c/readers-ab12/members");
    expect(await landing("/pick", [READERS])).toBe("/c/readers-ab12");
    // Without a club, the old pick page is Home.
    expect(await landing("/pick", [])).toBe("/");
    expect(await landing("/friends", [READERS])).toBe("/c/readers-ab12");
  });

  it("ignores a last-used club the reader is no longer in", async () => {
    expect(await landing("/club", [READERS], "gone-zz99")).toBe("/c/readers-ab12");
  });
});

/** Where a signed-out visit to `path` ends up. */
async function guestLanding(path: string) {
  vi.stubGlobal("localStorage", memoryStorage());
  setActivePinia(createPinia());
  me.mockRejectedValue(new ApiError("Not signed in", 401, "Not signed in"));
  const failure = await router.push(path).catch((error: unknown) => error);
  expect(failure).toBeUndefined();
  return router.currentRoute.value;
}

describe("old paths and the sign-in guard", () => {
  beforeEach(() => {
    me.mockReset();
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("sends the old feed and member links to the reader's own pages", async () => {
    expect(await landing("/club/grace", [READERS])).toBe("/u/grace");
    expect(await landing("/club/grace/shelf", [READERS])).toBe("/u/grace/shelf");
    expect(await landing("/friends/grace", [READERS])).toBe("/u/grace");
    expect(await landing("/library", [])).toBe("/discover");
  });

  it("keeps the invite code of the old sign-up path", async () => {
    const route = await guestLanding("/register?code=AB12");
    expect(route.path).toBe("/signup");
    expect(route.query).toEqual({ code: "AB12" });
  });

  it("sends a signed-out visit to the login page and back to the same place", async () => {
    const route = await guestLanding("/shelf?x=1");
    expect(route.path).toBe("/login");
    expect(route.query).toEqual({ next: "/shelf?x=1" });
  });

  it("opens the admin page to instance admins only", async () => {
    expect(await landing("/admin", [])).toBe("/");
    expect(await landing("/admin?tab=reports", [], undefined, true)).toBe("/admin?tab=reports");
  });

  it("sends a signed-in reader away from the guest pages", async () => {
    expect(await landing("/login", [])).toBe("/");
    expect(await landing("/signup?code=AB12", [])).toBe("/");
  });
});
