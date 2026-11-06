import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter, type Router } from "vue-router";
import type { User } from "../types";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  savePreferences: vi.fn(),
  adminSummary: vi.fn(),
  logout: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import { i18n } from "../i18n";
import { settingsHashTarget, settingsRoutes } from "../settingsRoutes";
import { useLocale } from "../stores/locale";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import { THEMES, useTheme } from "../stores/theme";
import SettingsPage from "./SettingsPage.vue";

const stub = { template: "<p>elsewhere</p>" };

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: { template: "<RouterView />" }, children: [...settingsRoutes, { path: "me", component: stub }, { path: "admin", component: stub }] },
      { path: "/login", component: stub },
    ],
  });
}

async function mountPage(path = "/settings", me: Partial<User> = {}, mail = true) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useService().$patch({ name: "Bookclub", legal: ["imprint", "privacy", "terms"], contact_email: null, mail });
  useSession().user = user({ username: "mia", email: "mia@example.com", ...me });
  const router = makeRouter();
  await router.push(path);
  // The row value's crossfade (out-in) would wait for a transitionend jsdom never fires.
  const wrapper = mount(SettingsPage, {
    attachTo: document.body,
    global: { plugins: [pinia, router], stubs: { transition: true } },
  });
  await flushPromises();
  return { wrapper, router };
}

const value = (wrapper: Awaited<ReturnType<typeof mountPage>>["wrapper"], row: string) =>
  wrapper.get(`.${row} .row-value`).text();

describe("SettingsPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.savePreferences.mockResolvedValue(user());
    api.adminSummary.mockResolvedValue({ target_hours: 72, reports: { open: 2, overdue: 0, oldest: null }, edit_requests: { open: 0, overdue: 0, oldest: null } });
    api.logout.mockResolvedValue(undefined);
    i18n.global.locale.value = "en";
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.unstubAllGlobals();
  });

  it("is a short list of rows that name their current value", async () => {
    const { wrapper } = await mountPage();
    expect(wrapper.find(".lede").exists()).toBe(false);
    expect(wrapper.findAll(".settings-row .row-label").map((label) => label.text())).toEqual([
      "Appearance",
      "Language",
      "Time zone",
      "Account",
      "Import",
      "Legal",
      "Log out",
    ]);
    expect(value(wrapper, "appearance-row")).toBe("Paper · System");
    expect(value(wrapper, "language-row")).toBe("English");
    expect(value(wrapper, "timezone-row")).toBe("UTC");
    expect(wrapper.find(".account-row .row-alert").exists()).toBe(false);
    expect(wrapper.get(".account-row").attributes("href")).toBe("/settings/account");
    expect(wrapper.get(".import-row").attributes("href")).toBe("/settings/import");
    // The photo, the favourites and the notification switches are not here.
    expect(wrapper.find("input[type=file]").exists()).toBe(false);
    expect(wrapper.find("input[type=checkbox]").exists()).toBe(false);
  });

  it("marks the account row while the address is unconfirmed or missing", async () => {
    let { wrapper } = await mountPage("/settings", { email_verified: false });
    expect(wrapper.get(".account-row .row-alert").text()).toBe("Not confirmed");
    ({ wrapper } = await mountPage("/settings", { email: null, email_verified: false }));
    expect(value(wrapper, "account-row")).toBe("No address");
    // A confirmed address with a new one waiting: the alert names the new one.
    ({ wrapper } = await mountPage("/settings", { email_verified: true, pending_email: "new@example.com" }));
    expect(wrapper.get(".account-row .row-alert").text()).toBe("Waiting for confirmation: new@example.com");
  });

  it("with mail off, asks for no confirmation the server could never send", async () => {
    for (const me of [{ email_verified: false }, { email_verified: true, pending_email: "new@example.com" }]) {
      const { wrapper } = await mountPage("/settings", me, false);
      expect(wrapper.find(".account-row .row-alert").exists()).toBe(false);
      wrapper.unmount();
    }
    const { wrapper } = await mountPage("/settings", { email: null, email_verified: false }, false);
    expect(wrapper.get(".account-row .row-alert").text()).toBe("No address");
  });

  it("leads back to the profile its link names, whatever page came before", async () => {
    window.history.replaceState({ back: "/shelf" }, "");
    try {
      const { wrapper, router } = await mountPage();
      const back = vi.spyOn(router, "back");
      const link = wrapper.get(".back-link a");
      expect(link.text()).toBe("← Profile");
      await link.trigger("click");
      await flushPromises();
      expect(back).not.toHaveBeenCalled();
      expect(router.currentRoute.value.path).toBe("/me");
    } finally {
      window.history.replaceState(null, "");
    }
  });

  it("shows the admin row with its count to instance admins only", async () => {
    let { wrapper } = await mountPage();
    expect(wrapper.find(".admin-link").exists()).toBe(false);
    expect(api.adminSummary).not.toHaveBeenCalled();
    ({ wrapper } = await mountPage("/settings", { is_admin: true }));
    expect(wrapper.get(".admin-link").attributes("href")).toBe("/admin");
    expect(wrapper.get(".admin-link .admin-count [aria-hidden=true]").text()).toBe("2");
  });

  it("changes mode and palette in a sheet, at once, and the row follows", async () => {
    const { wrapper } = await mountPage();
    const theme = useTheme();
    await wrapper.get(".appearance-row").trigger("click");
    const sheet = () => wrapper.get(".appearance-sheet");
    // matchMedia reports no dark preference here.
    expect(sheet().get(".mode-now").text()).toBe("Currently Light.");
    expect(sheet().get(".mode-now").classes()).not.toContain("hidden");

    const modes = sheet().findAll(".segmented button");
    expect(modes.map((button) => button.text())).toEqual(["Light", "Dark", "System"]);
    await modes[1]!.trigger("click");
    expect(theme.mode).toBe("dark");
    expect(modes[1]!.attributes("aria-pressed")).toBe("true");
    expect(sheet().get(".mode-now").classes()).toContain("hidden");

    const swatches = sheet().findAll(".swatch");
    expect(swatches.map((swatch) => swatch.get(".swatch-name").text())).toEqual(["Paper", "Slate", "Forest", "Ink"]);
    expect(swatches).toHaveLength(THEMES.length);
    // Each preview is painted in its own palette, in the mode now showing.
    expect(swatches[2]!.get(".swatch-preview").classes()).toContain("pal-forest-dark");
    await swatches[2]!.trigger("click");
    expect(theme.theme).toBe("forest");
    expect(swatches[2]!.attributes("aria-pressed")).toBe("true");
    expect(api.savePreferences).toHaveBeenCalledWith({ theme: "forest" });
    expect(value(wrapper, "appearance-row")).toBe("Forest · Dark");
  });

  it("picks the language in its sheet, which then closes", async () => {
    const { wrapper } = await mountPage();
    await wrapper.get(".language-row").trigger("click");
    const choices = wrapper.findAll(".language-sheet .choice-row");
    expect(choices.map((choice) => choice.text())).toEqual(["English", "Deutsch"]);
    await choices[1]!.trigger("click");
    await flushPromises();
    expect(useLocale().locale).toBe("de");
    expect(api.savePreferences).toHaveBeenCalledWith({ locale: "de" });
    expect(wrapper.find(".language-sheet").exists()).toBe(false);
    expect(value(wrapper, "language-row")).toBe("Deutsch");
    i18n.global.locale.value = "en";
  });

  it("opens the time zone in a sheet", async () => {
    const { wrapper } = await mountPage();
    await wrapper.get(".timezone-row").trigger("click");
    expect(wrapper.get(".timezone-sheet select").element).toBeInstanceOf(HTMLSelectElement);
  });

  it("opens the sheet a link asks for, and drops the query so Back does not reopen it", async () => {
    const { wrapper, router } = await mountPage("/settings?sheet=timezone");
    expect(wrapper.find(".timezone-sheet").exists()).toBe(true);
    expect(router.currentRoute.value.fullPath).toBe("/settings");
  });

  it("lists the legal texts and the contact in a sheet, and hides the row without either", async () => {
    let { wrapper } = await mountPage();
    useService().$patch({ contact_email: "legal@books.example" });
    await wrapper.get(".legal-row").trigger("click");
    const sheet = wrapper.get(".legal-sheet");
    expect(sheet.findAll(".legal-list a").map((link) => link.attributes("href"))).toEqual(["/imprint", "/privacy", "/terms"]);
    expect(sheet.get(".legal-contact a").attributes("href")).toBe("mailto:legal@books.example");

    ({ wrapper } = await mountPage());
    useService().$patch({ legal: [], contact_email: null });
    await flushPromises();
    expect(wrapper.find(".legal-row").exists()).toBe(false);
  });

  it("asks before logging out", async () => {
    const { wrapper, router } = await mountPage();
    await wrapper.get(".logout-row").trigger("click");
    const sheet = wrapper.get(".logout-sheet");
    expect(sheet.text()).toContain("Log out?");
    await sheet.get(".btn-ghost").trigger("click");
    expect(wrapper.find(".logout-sheet").exists()).toBe(false);
    expect(api.logout).not.toHaveBeenCalled();

    await wrapper.get(".logout-row").trigger("click");
    await wrapper.get(".logout-sheet .btn-danger").trigger("click");
    await flushPromises();
    expect(api.logout).toHaveBeenCalled();
    expect(useSession().user).toBeNull();
    expect(router.currentRoute.value.path).toBe("/login");
  });
});

describe("the settings routes", () => {
  it("send the old one-page anchors to their new place", async () => {
    const router = makeRouter();
    const cases: [string, string][] = [
      ["/settings#import", "/settings/import"],
      ["/settings#account-email", "/settings/account?sheet=email"],
      ["/settings#account-password", "/settings/account?sheet=password"],
      ["/settings#delete-account", "/settings/account?sheet=delete"],
      ["/settings#your-data", "/settings/account"],
      ["/settings#account-sessions", "/settings/devices"],
      ["/settings#appearance", "/settings?sheet=appearance"],
      ["/settings#language", "/settings?sheet=language"],
      ["/settings#timezone", "/settings?sheet=timezone"],
      ["/settings#legal", "/settings?sheet=legal"],
      ["/settings#favorites", "/me"],
      ["/settings#profile", "/me"],
      ["/settings#instance-admin", "/admin"],
      ["/settings#account-notifications", "/settings"],
      ["/settings", "/settings"],
      ["/settings/nothing-here", "/settings"],
    ];
    for (const [from, to] of cases) {
      await router.push("/login");
      await router.push(from);
      expect(router.currentRoute.value.fullPath, from).toBe(to);
    }
  });

  it("leave a plain visit alone", () => {
    expect(settingsHashTarget({ hash: "", query: {} })).toBe(true);
  });
});
