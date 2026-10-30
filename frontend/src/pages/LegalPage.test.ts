import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { LegalDoc, LegalDocumentOut, User } from "../types";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  legal: vi.fn(),
  myFavorites: vi.fn(),
  savePreferences: vi.fn(),
  // The shell asks for the Friends badge (#57).
  friends: vi.fn(),
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
import de from "../i18n/de.json";
import en from "../i18n/en.json";
import AppShell from "../layouts/AppShell.vue";
import { useLocale } from "../stores/locale";
import { LEGAL_DOCS, legalPath, useService } from "../stores/service";
import { useSession } from "../stores/session";
import LegalPage, { sanitizeLegalHtml } from "./LegalPage.vue";
import SettingsPage from "./SettingsPage.vue";

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

const ORIGIN = "https://books.example";

/** What the browser builds from the sanitised HTML, the way `v-html` sets it. */
function rendered(html: string): HTMLElement {
  const box = document.createElement("div");
  box.innerHTML = sanitizeLegalHtml(html, ORIGIN);
  return box;
}

/** Every element that could run script or load something, and every handler attribute. */
function dangerous(box: HTMLElement): string[] {
  const found = Array.from(
    box.querySelectorAll("script, style, iframe, object, embed, img, svg, math, form, input, link, meta, base, template"),
  ).map((element) => element.localName);
  for (const element of Array.from(box.querySelectorAll("*"))) {
    for (const { name, value } of Array.from(element.attributes)) {
      if (name.startsWith("on") || name === "style" || name === "src" || name === "srcdoc") found.push(`${name}=${value}`);
      if (name === "href" && !/^(https?:|mailto:|tel:|#|\/[^/])/i.test(value)) found.push(`href=${value}`);
    }
  }
  return found;
}

describe("sanitizeLegalHtml", () => {
  it("keeps the text, its structure and its links", () => {
    const html = [
      '<h1 id="top">Impressum</h1>',
      "<h2>Angaben gemäß § 5 DDG</h2>",
      "<p>Erika Mustermann<br>Musterstraße 1<br>12345 Berlin</p>",
      "<ul><li><strong>Telefon:</strong> <a href=\"tel:+4930123456\">030 123456</a></li>",
      '<li>E-Mail: <a href="mailto:kontakt@books.example">kontakt@books.example</a></li></ul>',
      '<table><thead><tr><th scope="col">Zweck</th></tr></thead><tbody><tr><td colspan="2">Konto</td></tr></tbody></table>',
      '<p lang="en"><em>Sources:</em> <a href="https://www.e-recht24.de">eRecht24</a>, <a href="#top">top</a>, <a href="/privacy">privacy</a></p>',
    ].join("");
    const box = rendered(html);
    expect(dangerous(box)).toEqual([]);
    expect(box.querySelector("h1")?.id).toBe("top");
    expect(box.querySelectorAll("br")).toHaveLength(2);
    expect(box.querySelector("th")?.getAttribute("scope")).toBe("col");
    expect(box.querySelector("td")?.getAttribute("colspan")).toBe("2");
    expect(box.querySelector("p[lang=en] em")?.textContent).toBe("Sources:");
    const links = Array.from(box.querySelectorAll("a")).map((a) => [
      a.getAttribute("href"),
      a.getAttribute("target"),
      a.getAttribute("rel"),
    ]);
    expect(links).toEqual([
      ["tel:+4930123456", null, null],
      ["mailto:kontakt@books.example", null, null],
      // Another site: a new tab, without an opener or a referrer.
      ["https://www.e-recht24.de", "_blank", "noopener noreferrer"],
      ["#top", null, null],
      ["/privacy", null, null],
    ]);
    expect(box.textContent).toContain("Musterstraße 1");
  });

  it("removes script, styles, frames, forms and pictures with everything in them", () => {
    const box = rendered(
      [
        "<p>Before</p>",
        "<script>window.hacked = true</script>",
        "<style>body { display: none }</style>",
        '<iframe src="https://evil.example"></iframe>',
        '<object data="x.swf"><p>inside object</p></object>',
        '<img src="https://tracker.example/pixel.gif" onerror="window.hacked = true">',
        '<picture><source srcset="https://tracker.example/a.webp"></picture>',
        '<form action="https://evil.example"><input name="password"><button>Send</button></form>',
        '<svg onload="window.hacked = true"><text>svg text</text></svg>',
        "<math><mi>x</mi></math>",
        "<template><p>template</p></template>",
        "<noscript><p>noscript</p></noscript>",
        "<!-- a comment -->",
        "<p>After</p>",
      ].join(""),
    );
    expect(dangerous(box)).toEqual([]);
    expect(box.innerHTML).toBe("<p>Before</p><p>After</p>");
  });

  it("removes event handlers, styles, classes and targets, and keeps the placeholder marker", () => {
    const box = rendered(
      '<aside class="legal-placeholder" data-legal-placeholder><p onclick="steal()" onmouseover="steal()" style="color:red" class="btn" title="t" dir="ltr">Text</p>' +
        '<a href="/terms" target="_top" ping="https://tracker.example">terms</a></aside>',
    );
    expect(box.innerHTML).toBe(
      '<aside data-legal-placeholder=""><p title="t" dir="ltr">Text</p><a href="/terms">terms</a></aside>',
    );
  });

  it("drops links to script, data and other schemes, however they are spelled", () => {
    const hrefs = [
      "javascript:alert(1)",
      "JaVaScRiPt:alert(1)",
      "  javascript:alert(1)",
      "java\tscript:alert(1)",
      "java&#x09;script:alert(1)",
      "&#106;avascript:alert(1)",
      "vbscript:msgbox(1)",
      "data:text/html,<script>alert(1)</script>",
      "file:///etc/passwd",
    ];
    for (const href of hrefs) {
      const box = rendered(`<a href="${href}">link</a>`);
      expect(box.querySelector("a")?.hasAttribute("href"), href).toBe(false);
      expect(box.textContent, href).toBe("link");
    }
  });

  it("unwraps elements it does not know and keeps their text", () => {
    const box = rendered('<center><font color="red">Old</font> <custom-tag onclick="x()">markup</custom-tag></center>');
    expect(box.innerHTML).toBe("Old markup");
  });

  it("unwraps a tag named like an Object.prototype member instead of keeping it", () => {
    // The parser accepts any tag name and lowercases it, so `constructor` is the one such name
    // that arrives. An allow-list lookup on a plain object kept it and then threw on its
    // attributes, which left the whole legal page blank.
    for (const tag of ["constructor", "Constructor"]) {
      expect(sanitizeLegalHtml(`<p><${tag} class="x" id="y">text</${tag}></p>`, ORIGIN), tag).toBe("<p>text</p>");
      expect(sanitizeLegalHtml(`<${tag}>bare</${tag}>`, ORIGIN), tag).toBe("bare");
    }
  });

  it("leaves nothing that turns into markup when the browser parses it again", () => {
    // Raw-text and RCDATA elements serialise their content unescaped; unwrapped, it stays text.
    const payloads = [
      "<xmp><img src=x onerror=alert(1)></xmp>",
      "<noembed><img src=x onerror=alert(1)></noembed>",
      "<plaintext><img src=x onerror=alert(1)>",
      '<noscript><p title="</noscript><img src=x onerror=alert(1)>"></noscript>',
      "<svg><style><img src=x onerror=alert(1)></style></svg>",
      "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)></style></mglyph></table></mtext></math>",
      '<p title="</p><img src=x onerror=alert(1)>">ok</p>',
      "<textarea><img src=x onerror=alert(1)></textarea>",
      "<title><img src=x onerror=alert(1)></title>",
    ];
    for (const payload of payloads) {
      const once = sanitizeLegalHtml(payload, ORIGIN);
      const box = rendered(payload);
      expect(dangerous(box), payload).toEqual([]);
      // Stable: sanitising the result again changes nothing.
      expect(sanitizeLegalHtml(once, ORIGIN), payload).toBe(once);
    }
  });

  it("keeps links to this app in the tab", () => {
    const box = rendered(`<a href="${ORIGIN}/privacy">privacy</a><a href="//evil.example/x">x</a>`);
    const [own, other] = Array.from(box.querySelectorAll("a"));
    expect(own?.hasAttribute("target")).toBe(false);
    expect(other?.getAttribute("target")).toBe("_blank");
  });
});

function atPath(root: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((acc, key) => {
    if (acc && typeof acc === "object" && !Array.isArray(acc)) {
      return (acc as Record<string, unknown>)[key];
    }
    return undefined;
  }, root);
}

describe("the legal documents", () => {
  it("has a page for each document", () => {
    expect(LEGAL_DOCS.map(legalPath)).toEqual(["/imprint", "/privacy", "/terms"]);
  });

  it("names every legal document, and the languages a text may fall back to, in both catalogs", () => {
    for (const catalog of [en, de]) {
      for (const doc of LEGAL_DOCS) {
        expect(typeof atPath(catalog, `legal.title.${doc}`), doc).toBe("string");
        expect(typeof atPath(catalog, `legal.short.${doc}`), doc).toBe("string");
      }
      for (const locale of ["en", "de"]) {
        expect(typeof atPath(catalog, `legal.onlyIn.${locale}`), locale).toBe("string");
      }
    }
    expect(de.legal.title.imprint).toBe("Impressum");
    expect(de.legal.title.privacy).toBe("Datenschutzerklärung");
  });
});

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

describe("the legal parts of the app shell and Settings", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.myFavorites.mockResolvedValue({ items: [] });
    api.friends.mockResolvedValue({ friends: [], incoming: [], outgoing: [], blocked: [], club_invites: [] });
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
