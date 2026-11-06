import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  myFavorites: vi.fn(),
  savePreferences: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import { i18n } from "../i18n";
import { THEMES, useTheme } from "../stores/theme";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import SettingsPage from "./SettingsPage.vue";

async function mountPage() {
  const pinia = createPinia();
  setActivePinia(pinia);
  useService().$patch({ name: "Bookclub", legal: ["imprint", "privacy", "terms"], contact_email: null });
  useSession().user = user({ username: "mia" });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/settings", component: SettingsPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push("/settings");
  const wrapper = mount(SettingsPage, {
    global: {
      plugins: [pinia, router],
      stubs: {
        AccountSettings: true,
        DeleteAccount: true,
        FavoritesEditor: true,
        GoodreadsImport: true,
        TimeZoneSetting: true,
        LanguageSwitch: true,
      },
    },
  });
  await flushPromises();
  return wrapper;
}

/** The paragraphs of one section, without the ones its stubbed children would render. */
function paragraphs(wrapper: Awaited<ReturnType<typeof mountPage>>, id: string) {
  return wrapper.get(`section[aria-labelledby=${id}]`).findAll("p").map((p) => p.text());
}

describe("SettingsPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.myFavorites.mockResolvedValue({ items: [] });
    api.savePreferences.mockResolvedValue(user());
    i18n.global.locale.value = "en";
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("goes straight to the controls where a heading says enough", async () => {
    const wrapper = await mountPage();
    expect(wrapper.get(".page-head .lede").text()).toBe("Signed in as mia.");
    // Profile: the photo and its buttons, no line about where it appears.
    expect(paragraphs(wrapper, "profile")).toEqual([]);
    expect(wrapper.find("section[aria-labelledby=profile] .profile-row").exists()).toBe(true);
    // Language: the switch alone.
    expect(paragraphs(wrapper, "language")).toEqual([]);
    // Legal: the three links, no line restating them.
    const legal = wrapper.get("section[aria-labelledby=legal]");
    expect(legal.findAll("p")).toHaveLength(0);
    expect(legal.findAll(".legal-list a")).toHaveLength(3);
    // Favourites still say where a favourite is added.
    expect(paragraphs(wrapper, "favorites")).toEqual(["Mark favourites on a book's page, shelved or not."]);
  });

  it("names the current mode only while following the device", async () => {
    const wrapper = await mountPage();
    const theme = useTheme();
    expect(theme.mode).toBe("system");
    const appearance = () => wrapper.get("section[aria-labelledby=appearance]");
    // matchMedia reports no dark preference here.
    expect(appearance().get(".mode-now").text()).toBe("Currently Light.");

    // The pressed Light or Dark button says it all.
    const buttons = appearance().findAll(".segmented button");
    await buttons[1].trigger("click");
    expect(buttons[1].attributes("aria-pressed")).toBe("true");
    expect(appearance().find(".mode-now").exists()).toBe(false);
    await buttons[0].trigger("click");
    expect(appearance().find(".mode-now").exists()).toBe(false);
    await buttons[2].trigger("click");
    expect(appearance().find(".mode-now").exists()).toBe(true);
  });

  it("shows each colour theme as its preview and name, without a description", async () => {
    const wrapper = await mountPage();
    const cards = wrapper.findAll(".theme-card");
    expect(cards).toHaveLength(THEMES.length);
    expect(cards.map((card) => card.get(".theme-name").text().replace("Active", "").trim())).toEqual([
      "Paper",
      "Slate",
      "Forest",
      "Ink",
    ]);
    for (const card of cards) {
      expect(card.find(".theme-preview").exists()).toBe(true);
      expect(card.find(".fine").exists()).toBe(false);
    }
    expect(THEMES.every((option) => !("blurbKey" in option))).toBe(true);
  });
});
