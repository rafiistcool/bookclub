// The #48 acceptance, through the real router, pages and API client: only
// `fetch` is replaced, by a small fake server that records every request.
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.vue";
import router from "./router";
import { LAST_CLUB_KEY } from "./stores/clubs";
import { activityItem, bookOut, clubOut, member, post, shelfItem, user, vote } from "./test/fixtures";
import { memoryStorage } from "./test/storage";
import type { ClubSummary, User } from "./types";

type Handler = (url: URL, body: unknown) => unknown;

/** A fake server: `"GET /api/shelf"` → the JSON it answers. Anything else is a 404 and noted. */
function fakeServer(routes: Record<string, Handler>) {
  const requests: string[] = [];
  const unknown: string[] = [];
  vi.stubGlobal("fetch", async (input: string, init: RequestInit = {}) => {
    const url = new URL(input, "http://localhost");
    const method = init.method ?? "GET";
    requests.push(`${method} ${url.pathname}${url.search}`);
    const key = `${method} ${url.pathname}`;
    const handler = Object.hasOwn(routes, key) ? routes[key] : undefined;
    if (!handler) unknown.push(key);
    const body = typeof init.body === "string" ? JSON.parse(init.body) : null;
    const answer = handler ? handler(url, body) : { detail: "Not found" };
    const status = handler ? (answer === undefined ? 204 : 200) : 404;
    return {
      status,
      ok: status < 400,
      text: async () => (answer === undefined ? "" : JSON.stringify(answer)),
    } as unknown as Response;
  });
  return { requests, unknown };
}

let wrapper: VueWrapper | null = null;

async function openApp(me: User, path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  wrapper = mount(App, { global: { plugins: [pinia, router], stubs: { BookCover: true } } });
  await router.push(path);
  await flushPromises();
  return wrapper;
}

async function go(path: string) {
  await router.push(path);
  await flushPromises();
}

const ADA_SHELF = {
  timezone: "UTC",
  user: { id: 1, username: "ada", avatar_url: null },
  items: [shelfItem({ id: 1, status: "currently_reading", progress: 30, book: { title: "Circe" } })],
  favorites: [],
};

describe("#48 acceptance", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("sessionStorage", memoryStorage());
    // The theme store asks for the colour scheme when the session arrives.
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    vi.unstubAllGlobals();
  });

  it("lets a reader with no clubs use the shelf, the diary and the quotes without a club call", async () => {
    const me = user({ clubs: [] });
    const own = post({ id: 5, author: "ada", mine: true, body: "Witches and pigs.", created_at: "2026-10-01T09:00:00Z" });
    const server = fakeServer({
      "GET /api/auth/me": () => me,
      "GET /api/shelf": () => ADA_SHELF,
      "GET /api/diary": () => ({
        items: [{ entry: own, book: bookOut({ id: 1, title: "Circe" }), parent_author: null, my_progress: 30, my_status: "currently_reading", club: null }],
        has_more: false,
        timezone: "UTC",
      }),
      "PATCH /api/posts/5": (_url, body) => ({ ...own, ...(body as object) }),
      "GET /api/quotes": () => ({
        items: [{
          id: 9, book: bookOut({ id: 1, title: "Circe" }), edition_id: null, author: "ada", mine: true,
          body: "It is a common fault in gods.", page: 12, visibility: "audience",
          created_at: "2026-10-02T09:00:00Z", created_label: "Fri 2 Oct 2026, 09:00 UTC",
        }],
      }),
    });

    const app = await openApp(me, "/shelf");
    expect(router.currentRoute.value.path).toBe("/shelf");
    expect(app.text()).toContain("Circe");

    await go("/diary");
    expect(app.get("h1").text()).toBe("Diary");
    expect(app.get(".feed-body").text()).toBe("Witches and pigs.");
    await app.get(".feed-only-me input").setValue(true);
    await flushPromises();
    expect(server.requests).toContain("PATCH /api/posts/5");
    expect(app.get(".only-me-badge").text()).toBe("Only you");

    await go("/quotes");
    expect(app.get("h1").text()).toBe("Quotes");
    expect(app.text()).toContain("It is a common fault in gods.");
    expect(server.requests).toContain("GET /api/quotes?username=ada");

    // Solo mode: nothing asked a club endpoint, and the Club tab leads to create-or-join.
    const clubCalls = server.requests.filter((line) => /\/api\/(clubs|activity|invites)/.test(line));
    expect(clubCalls).toEqual([]);
    expect(server.unknown).toEqual([]);
    await go("/club");
    expect(router.currentRoute.value.path).toBe("/clubs");
  });

  it("lets a reader in two clubs switch between them", async () => {
    const readers: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "admin", timezone: "Europe/Berlin", member_count: 3 };
    const owls: ClubSummary = { slug: "night-owls-cd34", name: "Night owls", role: "member", timezone: "UTC", member_count: 2 };
    const me = user({ clubs: [readers, owls] });
    const bySlug = (url: URL) => (url.pathname.includes(owls.slug) ? owls : readers);
    const feed = (url: URL) => ({
      items: [activityItem({ id: 1, kind: "nominated", actor: "grace", club: { slug: bySlug(url).slug, name: bySlug(url).name }, book: bookOut({ title: `${bySlug(url).name} book` }) })],
      has_more: false,
      timezone: bySlug(url).timezone,
    });
    const server = fakeServer({
      "GET /api/auth/me": () => me,
      "GET /api/clubs": () => ({ items: [clubOut({ ...readers }), clubOut({ ...owls })] }),
      "GET /api/shelf": () => ADA_SHELF,
      "GET /api/diary": () => ({ items: [], has_more: false, timezone: "UTC" }),
      "GET /api/activity": () => ({ items: [], has_more: false, timezone: "UTC" }),
      [`GET /api/clubs/${readers.slug}`]: () => clubOut({ ...readers }),
      [`GET /api/clubs/${owls.slug}`]: () => clubOut({ ...owls }),
      [`GET /api/clubs/${readers.slug}/activity`]: feed,
      [`GET /api/clubs/${owls.slug}/activity`]: feed,
      [`GET /api/clubs/${owls.slug}/members`]: () => ({
        items: [member({ username: "ada", you: true }), member({ username: "grace", role: "owner" })],
      }),
      [`GET /api/clubs/${owls.slug}/vote`]: () => vote({ timezone: "UTC", member_count: 2 }),
      [`GET /api/clubs/${owls.slug}/vote/suggestions`]: () => ({ items: [] }),
    });

    // Home shows both clubs.
    const app = await openApp(me, "/");
    expect(app.findAll(".club-card").map((card) => card.attributes("href"))).toEqual([
      `/c/${readers.slug}`,
      `/c/${owls.slug}`,
    ]);

    await go(`/c/${readers.slug}/events`);
    expect(app.get(".club-name").text()).toBe("Readers");
    expect(app.text()).toContain("Readers book");
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe(readers.slug);

    // The switcher keeps the section and asks the other club.
    await app.get(".club-switcher select").setValue(owls.slug);
    await flushPromises();
    expect(router.currentRoute.value.path).toBe(`/c/${owls.slug}/events`);
    expect(app.get(".club-name").text()).toBe("Night owls");
    expect(app.text()).toContain("Night owls book");
    expect(app.text()).not.toContain("Readers book");
    expect(server.requests).toContain(`GET /api/clubs/${owls.slug}/activity?limit=30`);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe(owls.slug);

    // Paths without a slug now lead into the last-used club.
    await go("/invites");
    expect(router.currentRoute.value.path).toBe(`/c/${owls.slug}/members`);
    expect(app.text()).toContain("grace");
    // A plain member of Night owls sees no invites; the admin of Readers would.
    expect(server.requests.some((line) => line.includes(`/api/clubs/${owls.slug}/invites`))).toBe(false);

    await go(`/c/${owls.slug}/vote`);
    expect(app.get(".club-switcher select").element).toHaveProperty("value", owls.slug);
    await app.get(".club-switcher select").setValue(readers.slug);
    await flushPromises();
    expect(router.currentRoute.value.path).toBe(`/c/${readers.slug}/vote`);
    expect(app.get(".club-name").text()).toBe("Readers");
  });
});
