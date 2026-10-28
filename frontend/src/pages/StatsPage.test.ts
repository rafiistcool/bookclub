import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { ClubStatsOut, PersonalStatsOut } from "../api/generated";
import { bookOut, clubOut, user } from "../test/fixtures";
import type { ClubSummary } from "../types";

const personalStats = vi.fn();
const clubStats = vi.fn();

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
  return {
    ApiError,
    api: {
      personalStats: (...args: unknown[]) => personalStats(...args),
      clubStats: (...args: unknown[]) => clubStats(...args),
    },
  };
});

import { ApiError } from "../api/client";
import { useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import ClubStatsPage from "./club/ClubStatsPage.vue";
import StatsPage from "./StatsPage.vue";

function personal(overrides: Partial<PersonalStatsOut> = {}): PersonalStatsOut {
  return {
    username: "ada", year: 2026, years: [2026, 2025], finished: 12, dnf: 1, pages: 3480,
    average_rating: 4.25, five_stars: 3, quotes: 7,
    top_book: bookOut({ id: 4, title: "Circe" }), longest_book: bookOut({ id: 5, title: "Middlemarch" }),
    by_month: [{ month: "2026-03", finished: 5 }, { month: "2026-04", finished: 2 }],
    ...overrides,
  };
}

async function mountAt(path: string, component: object, routePath: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: routePath, component }, { path: "/:rest(.*)*", component: { template: "<p />" } }],
  });
  await router.push(path);
  const wrapper = mount({ template: "<RouterView />" }, {
    global: { plugins: [router], stubs: { BookCover: true } },
  });
  await flushPromises();
  return { wrapper, router };
}

describe("year in books", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    personalStats.mockReset();
    clubStats.mockReset();
    useSession().user = user({ username: "ada" });
  });

  it("shows the reader's own year and switches years", async () => {
    personalStats.mockResolvedValue(personal());
    const { wrapper, router } = await mountAt("/stats", StatsPage, "/stats");
    expect(personalStats).toHaveBeenCalledWith({ username: undefined, year: undefined });
    expect(wrapper.get("h1").text()).toBe("Your year in books");
    const tiles = Object.fromEntries(
      wrapper.findAll(".stat-tile").map((tile) => [tile.attributes("data-stat"), tile.get("dd").text()]),
    );
    expect(tiles).toMatchObject({ finished: "12", pages: "3,480", average: "4.3", five: "3", dnf: "1", quotes: "7" });
    // Twelve bars; only the busiest month carries its number.
    expect(wrapper.findAll(".bar-col")).toHaveLength(12);
    expect(wrapper.findAll(".bar-col.peak")).toHaveLength(1);
    expect(wrapper.get(".bar-col.peak").attributes("title")).toBe("March: 5 finished");
    expect(wrapper.findAll(".highlight").map((row) => row.attributes("href"))).toEqual(["/book/4", "/book/5"]);

    personalStats.mockResolvedValue(personal({ year: 2025, finished: 0, dnf: 0, by_month: [] }));
    await wrapper.get(".year-field select").setValue("2025");
    await flushPromises();
    expect(router.currentRoute.value.query.year).toBe("2025");
    expect(personalStats).toHaveBeenLastCalledWith({ username: undefined, year: 2025 });
    expect(wrapper.text()).toContain("Nothing finished or set aside in 2025 yet.");
    expect(wrapper.find(".bar-col").exists()).toBe(false);
  });

  it("shows another reader's year, or the same not-found as for a stranger", async () => {
    personalStats.mockResolvedValue(personal({ username: "grace" }));
    const { wrapper } = await mountAt("/u/grace/stats", StatsPage, "/u/:username/stats");
    expect(personalStats).toHaveBeenCalledWith({ username: "grace", year: undefined });
    expect(wrapper.get("h1").text()).toBe("grace's year in books");

    personalStats.mockRejectedValue(new ApiError("Nobody with that name here", 404, "No member with that name"));
    const stranger = await mountAt("/u/max/stats", StatsPage, "/u/:username/stats");
    expect(stranger.wrapper.get(".error").text()).toBe("Nobody with that name here");
  });

  it("shows a club's year: members, picks and the best one", async () => {
    const readers: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "Europe/Berlin", member_count: 2 };
    useSession().user = user({ username: "ada", clubs: [readers] });
    useClubs().current = clubOut(readers);
    const pick = {
      pick_id: 3, book: bookOut({ id: 4, title: "Circe" }), set_by: "grace", started_at: "2026-02-01T10:00:00Z",
      ended_at: "2026-03-01T10:00:00Z", readers: 2, finished: 2, dnf: 0, average_rating: 4.5, ratings: [5, 4],
    };
    const stats: ClubStatsOut = {
      year: 2026, years: [2026], club_finished: 14, club_pages: 4000, club_average_rating: 4.1, best_pick: pick,
      by_month: [{ month: "2026-02", finished: 3 }], picks: [pick, { ...pick, pick_id: 4, average_rating: null, ended_at: null }],
      members: [
        { username: "grace", finished: 9, dnf: 0, pages: 2600, average_rating: 4, five_stars: 2, top_book: null, longest_book: null, quotes: 1 },
        { username: "ada", finished: 5, dnf: 1, pages: 1400, average_rating: null, five_stars: 0, top_book: null, longest_book: null, quotes: 0 },
      ],
    };
    clubStats.mockResolvedValue(stats);
    const { wrapper } = await mountAt("/c/readers-ab12/stats", ClubStatsPage, "/c/:slug/stats");
    expect(clubStats).toHaveBeenCalledWith("readers-ab12", undefined);
    expect(wrapper.get("h2").text()).toBe("The club's 2026");
    const rows = wrapper.findAll(".member-table tbody tr").map((row) => row.findAll("th, td").map((cell) => cell.text()));
    expect(rows).toEqual([
      ["grace", "9", "2,600", "4.0", "0"],
      ["ada", "5", "1,400", "–", "1"],
    ]);
    expect(wrapper.get(".member-table a").attributes("href")).toBe("/u/grace/stats?year=2026");
    const picks = wrapper.findAll(".pick-row");
    expect(picks[0].text()).toContain("Best rated");
    expect(picks[0].text()).toContain("1 Feb 2026 – 1 Mar 2026");
    expect(picks[1].text()).not.toContain("Best rated");
    expect(picks[1].text()).toContain("since 1 Feb 2026");
    expect(picks[1].text()).toContain("★ –");
    // One year only: no year picker.
    expect(wrapper.find(".year-field").exists()).toBe(false);
  });
});
