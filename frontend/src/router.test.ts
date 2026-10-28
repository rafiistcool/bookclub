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

import router from "./router";
import { LAST_CLUB_KEY } from "./stores/clubs";

const READERS = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };
const OWLS = { slug: "night-owls-cd34", name: "Night owls", role: "admin", timezone: "UTC", member_count: 2 };

async function landing(path: string, clubs: (typeof READERS)[], lastSlug?: string) {
  vi.stubGlobal("localStorage", memoryStorage());
  if (lastSlug) localStorage.setItem(LAST_CLUB_KEY, lastSlug);
  setActivePinia(createPinia());
  me.mockResolvedValue(user({ clubs }));
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
