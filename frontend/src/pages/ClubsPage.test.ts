import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { clubOut, user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";
import { clubSummary } from "../stores/clubs";

const clubs = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { clubs: (...args: unknown[]) => clubs(...args) },
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import ClubsPage from "./ClubsPage.vue";

async function mountPage(items = [clubOut()]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs: items.map(clubSummary) });
  clubs.mockResolvedValue({ items });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/clubs", component: ClubsPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push("/clubs");
  const wrapper = mount(ClubsPage, { global: { plugins: [pinia, router, i18n], stubs: { ClubCard: true } } });
  await flushPromises();
  return { wrapper, router };
}

describe("ClubsPage", () => {
  beforeEach(() => {
    clubs.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("lists the clubs under the heading alone, with no lede", async () => {
    const { wrapper } = await mountPage();
    expect(wrapper.findAll(".club-grid li")).toHaveLength(1);
    expect(wrapper.find(".page-head .lede").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Every club you're in");
  });

  it("says once that there is no club yet, and offers both ways in", async () => {
    const { wrapper } = await mountPage([]);
    expect(clubs).not.toHaveBeenCalled();
    expect(wrapper.get(".page-head .lede").text()).toBe("You're not in a club yet.");
    expect(wrapper.find(".club-grid").exists()).toBe(false);
    expect(wrapper.get('.create-way a[href="/clubs/new"]').text()).toBe("Create a club");
    expect(wrapper.get(".create-way").text()).toContain("Up to 20 members.");
    expect(wrapper.find("form input[name=invite]").exists()).toBe(true);
  });

  it("names the create action once: the button, with no heading over it", async () => {
    i18n.global.locale.value = "de";
    try {
      const { wrapper } = await mountPage([]);
      const panel = wrapper.get(".create-way");
      expect(panel.find("h2").exists()).toBe(false);
      expect(panel.text().match(/Club gründen/g)).toHaveLength(1);
    } finally {
      i18n.global.locale.value = "en";
    }
  });

  it("opens the join page with the code", async () => {
    const { wrapper, router } = await mountPage([]);
    await wrapper.get("input[name=invite]").setValue(" 7k3m9qx2hd ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(router.currentRoute.value.path).toMatch(/^\/join\//);
  });
});
