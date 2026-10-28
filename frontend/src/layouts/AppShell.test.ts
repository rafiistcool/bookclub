import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";
import type { LegalDoc, User } from "../types";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  myFavorites: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import SettingsPage from "../pages/SettingsPage.vue";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import AppShell from "./AppShell.vue";

interface Shell {
  path?: string;
  me?: User;
  legal?: LegalDoc[];
  contact?: string | null;
}

/**
 * The real AppShell with a stand-in page, and the real Settings page (its
 * heavy sections stubbed) at `/settings`: the shell is where the prompts and
 * the legal footer live, so only a mount of the shell shows they are there.
 */
async function mountShell({ path = "/", me = user(), legal = ["imprint", "privacy", "terms"], contact = null }: Shell = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useService().$patch({ name: "Bookclub", legal, terms_version: "2026-11-01", contact_email: contact });
  useSession().user = me;
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: "/",
        component: AppShell,
        children: [
          { path: "", component: { template: '<p class="page-home">home</p>' } },
          { path: "settings", component: SettingsPage },
        ],
      },
      { path: "/login", component: { template: "<p>login</p>" } },
    ],
  });
  await router.push(path);
  const wrapper = mount(
    { template: "<RouterView />" },
    {
      global: {
        plugins: [pinia, router],
        stubs: {
          AccountSettings: true,
          DeleteAccount: true,
          FavoritesEditor: true,
          GoodreadsImport: true,
          TimeZoneSetting: true,
        },
      },
    },
  );
  await flushPromises();
  return wrapper;
}

describe("AppShell", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.myFavorites.mockResolvedValue({ items: [] });
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("asks for the terms above the email prompt, before the page", async () => {
    const wrapper = await mountShell({
      me: user({ needs_terms: true, needs_email_prompt: true, email: null, email_verified: false }),
    });
    const main = wrapper.get("main");
    const order = Array.from(main.element.children).map((element) => element.className);
    expect(order.findIndex((name) => name.includes("terms-prompt"))).toBe(0);
    expect(order.findIndex((name) => name.includes("email-prompt"))).toBe(1);
    expect(order.findIndex((name) => name.includes("page-home"))).toBe(2);
    expect(wrapper.get(".terms-prompt").text()).toContain("Accept the terms");
  });

  it("shows neither prompt to an account that needs neither", async () => {
    const wrapper = await mountShell();
    expect(wrapper.find(".terms-prompt").exists()).toBe(false);
    expect(wrapper.find(".email-prompt").exists()).toBe(false);
    expect(wrapper.find(".page-home").exists()).toBe(true);
  });

  it("ends every page with the legal links the server serves", async () => {
    let wrapper = await mountShell();
    const footer = wrapper.get("main > footer.shell-footer");
    expect(footer.findAll("nav.legal-links a").map((link) => [link.attributes("href"), link.text()])).toEqual([
      ["/imprint", "Imprint"],
      ["/privacy", "Privacy"],
      ["/terms", "Terms"],
    ]);

    wrapper = await mountShell({ legal: ["imprint"] });
    expect(wrapper.findAll("footer nav.legal-links a").map((link) => link.attributes("href"))).toEqual(["/imprint"]);

    wrapper = await mountShell({ legal: [] });
    expect(wrapper.find("footer.shell-footer").exists()).toBe(false);
  });

  it("drops the footer on Settings, whose Legal section lists the texts and the contact", async () => {
    const wrapper = await mountShell({ path: "/settings", contact: "legal@books.example" });
    expect(wrapper.find("footer.shell-footer").exists()).toBe(false);
    expect(wrapper.find("nav.legal-links").exists()).toBe(false);
    const section = wrapper.get("section[aria-labelledby=legal]");
    expect(section.get("h2").text()).toBe("Legal");
    expect(section.findAll(".legal-list a").map((link) => [link.attributes("href"), link.text()])).toEqual([
      ["/imprint", "Imprint"],
      ["/privacy", "Privacy policy"],
      ["/terms", "Terms of use"],
    ]);
    const mail = section.get(".legal-contact a");
    expect(mail.attributes("href")).toBe("mailto:legal@books.example");
    expect(mail.text()).toBe("legal@books.example");
  });

  it("keeps the Legal section for the contact alone, and drops it without texts and contact", async () => {
    let wrapper = await mountShell({ path: "/settings", legal: [], contact: "legal@books.example" });
    let section = wrapper.get("section[aria-labelledby=legal]");
    expect(section.find(".legal-list").exists()).toBe(false);
    expect(section.get(".legal-contact a").attributes("href")).toBe("mailto:legal@books.example");

    wrapper = await mountShell({ path: "/settings", legal: ["privacy"] });
    section = wrapper.get("section[aria-labelledby=legal]");
    expect(section.findAll(".legal-list a").map((link) => link.attributes("href"))).toEqual(["/privacy"]);
    expect(section.find(".legal-contact").exists()).toBe(false);

    wrapper = await mountShell({ path: "/settings", legal: [] });
    expect(wrapper.find("section[aria-labelledby=legal]").exists()).toBe(false);
  });

  it("links the privacy policy from the email prompt only when the server has one", async () => {
    const me = user({ needs_email_prompt: true, email: null, email_verified: false });
    let wrapper = await mountShell({ me });
    const prompt = wrapper.get(".email-prompt");
    expect(prompt.findAll("a").map((link) => [link.attributes("href"), link.text()])).toEqual([
      ["/privacy", "privacy policy"],
    ]);

    wrapper = await mountShell({ me, legal: ["imprint", "terms"] });
    expect(wrapper.get(".email-prompt").findAll("a")).toHaveLength(0);
  });
});
