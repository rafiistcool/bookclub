import { afterEach, describe, expect, it } from "vitest";
import { i18n } from "./i18n";
import { translateApiError } from "./i18n/apiErrors";
import { isTermsChanged, legalPath, LEGAL_DOCS, sanitizeLegalHtml, TERMS_CHANGED } from "./legal";

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

describe("the legal documents", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  it("has a page for each document", () => {
    expect(LEGAL_DOCS.map(legalPath)).toEqual(["/imprint", "/privacy", "/terms"]);
  });

  it("recognises the server's answer that the terms changed, and translates it", () => {
    expect(isTermsChanged({ status: 400, detail: TERMS_CHANGED })).toBe(true);
    expect(isTermsChanged({ status: 400, detail: "That invite code is not valid" })).toBe(false);
    expect(isTermsChanged(new Error(TERMS_CHANGED))).toBe(false);
    expect(isTermsChanged(null)).toBe(false);
    expect(translateApiError(TERMS_CHANGED)).toBe("The terms of use have changed. Have a look at them and try again.");
    i18n.global.locale.value = "de";
    expect(translateApiError(TERMS_CHANGED)).toBe(
      "Die Nutzungsbedingungen haben sich geändert. Sieh sie dir an und versuch es nochmal.",
    );
  });
});
