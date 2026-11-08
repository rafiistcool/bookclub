import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter, RouterLink } from "vue-router";
import { clubOut, user } from "../test/fixtures";
import type { ClubSummary } from "../types";
import { memoryStorage } from "../test/storage";

const club = vi.fn();

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
  return { ApiError, api: { club: (...args: unknown[]) => club(...args) } };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { LAST_CLUB_KEY, useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import ClubLayout from "./ClubLayout.vue";

const READERS: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "admin", timezone: "Europe/Berlin", member_count: 3 };
const OWLS: ClubSummary = { slug: "night-owls-cd34", name: "Night owls", role: "member", timezone: "UTC", member_count: 2 };

async function mountAt(path: string, clubs: ClubSummary[] = [READERS, OWLS]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: "/c/:slug",
        component: ClubLayout,
        children: [
          { path: "", component: { template: '<p class="section-home">home</p>' } },
          { path: "vote", component: { template: '<p class="section-vote">vote</p>' } },
          { path: "members", component: { template: '<p class="section-members">members</p>' } },
          { path: "overlap", component: { template: '<p class="section-overlap">overlap</p>' } },
          { path: "stats", component: { template: '<p class="section-stats">stats</p>' } },
        ],
      },
      { path: "/clubs", component: { template: "<p>clubs</p>" } },
      { path: "/clubs/new", component: { template: "<p>new club</p>" } },
    ],
  });
  await router.push(path);
  // The real RouterLink: the sheets' rows navigate.
  const wrapper = mount(
    { template: "<RouterView />" },
    { global: { plugins: [pinia, router, i18n], stubs: { RouterLink } } },
  );
  await flushPromises();
  return { wrapper, router };
}

describe("ClubLayout", () => {
  beforeEach(() => {
    club.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("shows the section once the club answered and remembers the club", async () => {
    club.mockResolvedValue(clubOut({ ...READERS, current_pick: null }));
    const { wrapper } = await mountAt("/c/readers-ab12/vote");
    expect(club).toHaveBeenCalledWith("readers-ab12");
    expect(wrapper.find(".section-vote").exists()).toBe(true);
    expect(wrapper.get(".club-name").text()).toBe("Readers");
    // The name stands alone: no "Club" kicker above it and no meta line under it.
    expect(wrapper.find(".club-head .kicker").exists()).toBe(false);
    expect(wrapper.get(".club-head").text()).not.toContain("Admin");
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("readers-ab12");
    expect(wrapper.get(".club-tab.active").text()).toBe("Vote");
    expect(wrapper.get(".club-tab.active").attributes("aria-current")).toBe("page");
    // Four short tabs and "More", nothing else.
    expect(wrapper.findAll(".club-tab").map((tab) => tab.text())).toEqual(["Book", "Talk", "Vote", "Group", "More"]);
    expect(wrapper.get(".club-tabs").attributes("style")).toContain("--tab: 2");
  });

  it("opens the club sheet from the name and switches club, keeping the section", async () => {
    club.mockImplementation((slug: string) =>
      Promise.resolve(clubOut(slug === READERS.slug ? READERS : OWLS)),
    );
    const { wrapper, router } = await mountAt("/c/readers-ab12/vote");
    const name = wrapper.get(".club-name-btn");
    expect(name.attributes("aria-haspopup")).toBe("dialog");
    expect(name.attributes("aria-expanded")).toBe("false");
    await name.trigger("click");
    expect(name.attributes("aria-expanded")).toBe("true");
    const rows = wrapper.findAll(".sheet .club-choice");
    expect(rows.map((row) => row.find(".switch-name").text())).toEqual(["Readers", "Night owls"]);
    // The role and the member count moved from the header into the sheet.
    expect(rows[0]!.text()).toContain("3 members · Admin");
    expect(rows[0]!.attributes("aria-current")).toBe("true");
    expect(rows[1]!.attributes("href")).toBe("/c/night-owls-cd34/vote");
    expect(wrapper.findAll(".sheet .switch-more a").map((row) => row.attributes("href"))).toEqual([
      "/clubs",
      "/clubs/new",
    ]);
    await rows[1]!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/c/night-owls-cd34/vote");
    expect(wrapper.find(".sheet").exists()).toBe(false);
    expect(wrapper.get(".club-name").text()).toBe("Night owls");
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("night-owls-cd34");
  });

  it("offers the club sheet with a single club too", async () => {
    club.mockResolvedValue(clubOut(OWLS));
    const { wrapper } = await mountAt("/c/night-owls-cd34", [OWLS]);
    await wrapper.get(".club-name-btn").trigger("click");
    expect(wrapper.findAll(".sheet .club-choice")).toHaveLength(1);
    expect(wrapper.find('.sheet a[href="/clubs"]').exists()).toBe(true);
  });

  it("keeps the quieter sections behind More and marks it there", async () => {
    club.mockResolvedValue(clubOut(READERS));
    const { wrapper, router } = await mountAt("/c/readers-ab12/stats");
    const more = wrapper.get(".more-tab");
    expect(more.classes()).toContain("active");
    expect(more.attributes("aria-label")).toBe("More, now: Year");
    expect(wrapper.get(".club-tabs").attributes("style")).toContain("--tab: 4");
    await more.trigger("click");
    const links = wrapper.findAll(".sheet .more-row");
    expect(links.map((row) => row.attributes("href"))).toEqual([
      "/c/readers-ab12/overlap",
      "/c/readers-ab12/events",
      "/c/readers-ab12/stats",
      "/c/readers-ab12/members#invites",
      "/c/readers-ab12/members#club-settings",
    ]);
    expect(links[2]!.attributes("aria-current")).toBe("page");
    await links[0]!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12/overlap");
    expect(wrapper.find(".sheet").exists()).toBe(false);
  });

  it("closes the sheets on Back between two sections of one page", async () => {
    club.mockResolvedValue(clubOut(READERS));
    const { wrapper, router } = await mountAt("/c/readers-ab12/members#invites");
    await router.push("/c/readers-ab12/members#club-settings");
    await flushPromises();
    await wrapper.get(".more-tab").trigger("click");
    expect(wrapper.find(".sheet").exists()).toBe(true);
    router.back();
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/c/readers-ab12/members#invites");
    expect(wrapper.find(".sheet").exists()).toBe(false);

    // A query alone keeps a sheet open.
    await wrapper.get(".more-tab").trigger("click");
    await router.replace("/c/readers-ab12/members?year=2025#invites");
    await flushPromises();
    expect(wrapper.find(".sheet").exists()).toBe(true);
  });

  it("shows members no admin rows behind More", async () => {
    club.mockResolvedValue(clubOut(OWLS));
    const { wrapper } = await mountAt("/c/night-owls-cd34");
    expect(wrapper.get(".club-tab.active").text()).toBe("Book");
    expect(wrapper.get(".more-tab").classes()).not.toContain("active");
    await wrapper.get(".more-tab").trigger("click");
    expect(wrapper.findAll(".sheet .more-row")).toHaveLength(3);
    expect(wrapper.find('.sheet a[href$="#invites"]').exists()).toBe(false);
  });

  it("shows 'club not found' on a 404 and drops the stale slug", async () => {
    localStorage.setItem(LAST_CLUB_KEY, "readers-ab12");
    club.mockRejectedValue(new ApiError("That club does not exist", 404, "No such club"));
    const { wrapper } = await mountAt("/c/readers-ab12");
    expect(wrapper.find(".club-missing").exists()).toBe(true);
    expect(wrapper.find(".section-home").exists()).toBe(false);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBeNull();
    expect(useSession().user?.clubs.map((row) => row.slug)).toEqual(["night-owls-cd34"]);
    expect(useClubs().goneSlug).toBe("readers-ab12");
  });

  it("keeps the page on other errors and offers a retry", async () => {
    club.mockRejectedValueOnce(new ApiError("Something went wrong", 500, "boom"));
    const { wrapper } = await mountAt("/c/readers-ab12");
    expect(wrapper.find(".club-missing").exists()).toBe(false);
    expect(wrapper.get(".error").text()).toContain("Something went wrong");
    club.mockResolvedValue(clubOut(READERS));
    await wrapper.get(".error button").trigger("click");
    await flushPromises();
    expect(wrapper.find(".section-home").exists()).toBe(true);
  });
});
