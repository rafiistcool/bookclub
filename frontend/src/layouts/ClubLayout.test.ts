import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
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

async function mountAt(path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs: [READERS, OWLS] });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: "/c/:slug",
        component: ClubLayout,
        children: [
          { path: "", component: { template: '<p class="section-home">home</p>' } },
          { path: "vote", component: { template: '<p class="section-vote">vote</p>' } },
        ],
      },
      { path: "/clubs", component: { template: "<p>clubs</p>" } },
    ],
  });
  await router.push(path);
  const wrapper = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, router, i18n] } });
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
    expect(wrapper.text()).toContain("Admin");
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("readers-ab12");
    expect(wrapper.get(".club-tab.active").text()).toBe("Vote");
    // Two clubs: the switcher offers both.
    expect(wrapper.findAll(".club-switcher option").map((row) => row.text())).toEqual(["Readers", "Night owls"]);
  });

  it("switches club and keeps the section", async () => {
    club.mockImplementation((slug: string) =>
      Promise.resolve(clubOut(slug === READERS.slug ? READERS : OWLS)),
    );
    const { wrapper, router } = await mountAt("/c/readers-ab12/vote");
    await wrapper.get(".club-switcher select").setValue("night-owls-cd34");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/c/night-owls-cd34/vote");
    expect(wrapper.get(".club-name").text()).toBe("Night owls");
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("night-owls-cd34");
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
