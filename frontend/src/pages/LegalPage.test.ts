import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { LegalDoc, LegalDocumentOut } from "../types";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  legal: vi.fn(),
  savePreferences: vi.fn(),
};

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
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useLocale } from "../stores/locale";
import { useService } from "../stores/service";
import LegalPage from "./LegalPage.vue";

function text(overrides: Partial<LegalDocumentOut> = {}): LegalDocumentOut {
  return {
    doc: "imprint",
    locale: "en",
    fallback: false,
    updated: "2026-11-01",
    html: "<h2>Imprint</h2><p>Operator</p>",
    ...overrides,
  };
}

async function mountLegal(doc: LegalDoc, options: { locale?: "en" | "de"; contact?: string | null } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useService().$patch({ legal: ["imprint", "privacy", "terms"], contact_email: options.contact ?? null });
  useLocale().locale = options.locale ?? "en";
  i18n.global.locale.value = options.locale ?? "en";
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/imprint", component: { template: "<p>imprint</p>" } },
      { path: "/privacy", component: { template: "<p>privacy</p>" } },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push(`/${doc}`);
  const wrapper = mount(LegalPage, {
    props: { doc },
    attachTo: document.body,
    global: { plugins: [pinia, router, i18n] },
  });
  await flushPromises();
  return { wrapper, router };
}

describe("LegalPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.savePreferences.mockResolvedValue(user());
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => {
    document.body.innerHTML = "";
    i18n.global.locale.value = "en";
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("shows the operator's text in the reader's language, without its script", async () => {
    api.legal.mockResolvedValue(
      text({
        html:
          '<h2>Imprint</h2><p onclick="window.hacked = true">Erika Mustermann</p>' +
          "<script>window.hacked = true</script>" +
          '<img src="https://tracker.example/pixel.gif" onerror="window.hacked = true">' +
          '<a href="javascript:alert(1)">bad link</a>',
      }),
    );
    const { wrapper } = await mountLegal("imprint");
    expect(api.legal).toHaveBeenCalledWith("imprint", "en");
    const article = wrapper.get("article.legal-text");
    expect(article.attributes("lang")).toBe("en");
    expect(article.attributes("aria-label")).toBe("Imprint");
    expect(article.get("h2").text()).toBe("Imprint");
    expect(article.get("p").attributes("onclick")).toBeUndefined();
    expect(article.find("script").exists()).toBe(false);
    expect(article.find("img").exists()).toBe(false);
    expect(article.get("a").attributes("href")).toBeUndefined();
    expect(wrapper.find(".legal-note").exists()).toBe(false);
    expect(wrapper.get(".legal-updated").text()).toBe("Last updated 1 Nov 2026");
    // The other texts are a tap away.
    expect(wrapper.get("nav.legal-links").findAll("a").map((a) => a.attributes("href"))).toEqual([
      "/imprint",
      "/privacy",
      "/terms",
    ]);
  });

  it("says when the text is only in the other language", async () => {
    api.legal.mockResolvedValue(text({ doc: "privacy", locale: "en", fallback: true, updated: null }));
    const { wrapper } = await mountLegal("privacy", { locale: "de" });
    expect(api.legal).toHaveBeenCalledWith("privacy", "de");
    expect(wrapper.get(".legal-note").text()).toBe("Diesen Text gibt es nur auf Englisch.");
    expect(wrapper.get("article").attributes("lang")).toBe("en");
    expect(wrapper.find(".legal-updated").exists()).toBe(false);
  });

  it("loads the other language when the reader switches", async () => {
    api.legal.mockResolvedValueOnce(text()).mockResolvedValueOnce(
      text({ locale: "de", html: "<h2>Impressum</h2><p>Betreiberin</p>" }),
    );
    const { wrapper } = await mountLegal("imprint");
    const de = wrapper.findAll(".segmented button").find((button) => button.text() === "DE");
    await de!.trigger("click");
    await flushPromises();
    expect(api.legal).toHaveBeenLastCalledWith("imprint", "de");
    expect(wrapper.get("article h2").text()).toBe("Impressum");
    expect(wrapper.get("article").attributes("lang")).toBe("de");
  });

  it("says that a text is not online yet, and names the contact", async () => {
    api.legal.mockRejectedValue(new ApiError("Not found", 404, "Not found"));
    const { wrapper } = await mountLegal("terms", { contact: "kontakt@books.example" });
    expect(wrapper.get("h1").text()).toBe("Terms of use");
    expect(wrapper.text()).toContain("This text is not online yet.");
    expect(wrapper.find("article").exists()).toBe(false);
    expect(wrapper.find(".legal-page button.btn").exists()).toBe(false);
    expect(wrapper.get(".legal-contact a").attributes("href")).toBe("mailto:kontakt@books.example");
  });

  it("offers to try again when the text did not load", async () => {
    api.legal.mockRejectedValueOnce(new ApiError("The server is busy", 503, "busy")).mockResolvedValueOnce(text());
    const { wrapper } = await mountLegal("imprint");
    expect(wrapper.get(".error").text()).toBe("Could not load this text.");
    await wrapper.get(".btn-row button").trigger("click");
    await flushPromises();
    expect(api.legal).toHaveBeenCalledTimes(2);
    expect(wrapper.get("article h2").text()).toBe("Imprint");
  });

  it("opens the app's own links in the app and scrolls to sections within the text", async () => {
    api.legal.mockResolvedValue(
      text({
        html:
          '<p><a href="/privacy">privacy policy</a> <a href="#contact">contact</a> <a href="https://www.e-recht24.de">source</a></p>' +
          '<h3 id="contact">Contact</h3>',
      }),
    );
    const scrolled: Element[] = [];
    // jsdom has no layout, so no scrollIntoView of its own.
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this);
    };
    onTestFinished(() => {
      Element.prototype.scrollIntoView = original;
    });
    const { wrapper, router } = await mountLegal("imprint");

    await wrapper.get("a[href='#contact']").trigger("click");
    expect(scrolled.map((element) => element.id)).toEqual(["contact"]);
    expect(router.currentRoute.value.path).toBe("/imprint");

    const external = wrapper.get("a[href='https://www.e-recht24.de']");
    expect(external.attributes("target")).toBe("_blank");
    expect(external.attributes("rel")).toBe("noopener noreferrer");

    await wrapper.get("a[href='/privacy']").trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/privacy");
  });
});
