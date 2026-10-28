import { describe, expect, it, vi } from "vitest";

/**
 * The browser contacts no third party: the privacy text says so, and the
 * shell's Content-Security-Policy (server/src/spa.rs) blocks it at runtime.
 * This test finds such a load in the sources, before anyone opens a page.
 *
 * It reads the sources, never dist/: CI runs `npm test` before `npm run build`.
 * Checked are `index.html`, every text file in `public/` and
 * `src/**\/*.{css,vue,ts}` without the tests. A load is what the browser
 * fetches by itself: `url(…)`, `@import`, `<link>`, `<script>` and the media
 * elements' `src`, `srcset` and `poster`, the icons of the manifest, and
 * `fetch`, `WebSocket`, `EventSource`, workers and beacons. A link the reader
 * clicks (`<a href>`, such as a book's Open Library page) is not a load.
 */

type Fs = {
  readFileSync(path: URL, encoding: "utf8"): string;
  readdirSync(path: URL, options: { recursive: true }): string[];
};

/** The frontend directory (a variable: Vite rewrites `new URL` of a literal). */
const ROOT = "../";

async function fs(): Promise<Fs> {
  return vi.importActual<Fs>("node:fs");
}

async function read(path: string): Promise<string> {
  return (await fs()).readFileSync(new URL(ROOT + path, import.meta.url), "utf8");
}

/** Every file below `dir` (relative to the frontend) whose name matches. */
async function files(dir: string, name: RegExp): Promise<string[]> {
  const found = (await fs()).readdirSync(new URL(ROOT + dir, import.meta.url), { recursive: true });
  return found
    .map((path) => `${dir}${path.replaceAll("\\", "/")}`)
    .filter((path) => name.test(path))
    .sort();
}

/** An absolute or protocol-relative URL: one that names an origin. */
function isExternal(url: string): boolean {
  return /^(?:[a-z][a-z0-9+.-]*:)?\/\//i.test(url.trim());
}

const LOAD_TAG = /<(link|script|img|source|iframe|video|audio|track|embed|object|image|use|input)\b[^>]*>/gi;
const LOAD_ATTRIBUTE = /(?:^|\s)(href|xlink:href|src|srcset|poster|data)\s*=\s*(["'])(.*?)\2/gi;
const CSS_URL = /url\(\s*(["']?)([^"')\s]+)\1\s*\)/gi;
const CSS_IMPORT = /@import\s+(?:url\(\s*)?(["']?)([^"')\s;]+)/gi;
const SCRIPT_LOAD =
  /(?:\bfetch|\bnew\s+(?:WebSocket|EventSource|Worker|SharedWorker)|\.sendBeacon|\bimportScripts|\bimport)\s*\(\s*([`'"])(.*?)\1/g;

/** The URLs a text loads by itself, as `<kind> <url>`. */
function loads(text: string): string[] {
  const found: string[] = [];
  for (const tag of text.matchAll(LOAD_TAG)) {
    for (const attribute of tag[0].matchAll(LOAD_ATTRIBUTE)) {
      const urls = attribute[1] === "srcset" ? attribute[3].split(",").map((part) => part.trim().split(/\s+/)[0]) : [attribute[3]];
      found.push(...urls.map((url) => `<${tag[1].toLowerCase()} ${attribute[1]}> ${url}`));
    }
  }
  for (const match of text.matchAll(CSS_URL)) found.push(`url() ${match[2]}`);
  for (const match of text.matchAll(CSS_IMPORT)) found.push(`@import ${match[2]}`);
  for (const match of text.matchAll(SCRIPT_LOAD)) found.push(`script ${match[2]}`);
  return found;
}

/** The loads of `text` that leave the origin. */
function externalLoads(text: string): string[] {
  return loads(text).filter((load) => isExternal(load.slice(load.lastIndexOf(" ") + 1)));
}

describe("the scanner", () => {
  it("finds every kind of load and no click-only link", () => {
    const sample = `
      <link rel="stylesheet" href="https://fonts.example/a.css">
      <script src="//cdn.example/x.js"></script>
      <img alt="" srcset="/a.png 1x, https://img.example/b.png 2x">
      <a href="https://openlibrary.org/works/OL1W">Open Library</a>
      <svg><use xlink:href="https://sprites.example/s.svg#i" /></svg>
      .a { background: url("https://img.example/c.png"); mask: url(#local); }
      @import "https://fonts.example/d.css";
      @import "./tokens.css";
      fetch("https://api.example/e"); new WebSocket('wss://ws.example/f');
      navigator.sendBeacon(\`https://beacon.example/g\`);
      fetch("/api/shelf"); img.src = data;
    `;
    expect(externalLoads(sample)).toEqual([
      "<link href> https://fonts.example/a.css",
      "<script src> //cdn.example/x.js",
      "<img srcset> https://img.example/b.png",
      "<use xlink:href> https://sprites.example/s.svg#i",
      "url() https://img.example/c.png",
      "@import https://fonts.example/d.css",
      "script https://api.example/e",
      "script wss://ws.example/f",
      "script https://beacon.example/g",
    ]);
    expect(loads(sample)).toContain("url() #local");
    expect(loads(sample)).toContain("@import ./tokens.css");
    expect(loads(sample)).toContain("script /api/shelf");
  });
});

describe("third parties", () => {
  it("index.html loads nothing from another origin", async () => {
    expect(externalLoads(await read("index.html"))).toEqual([]);
  });

  it("the manifest names no other origin", async () => {
    const manifest: unknown = JSON.parse(await read("public/manifest.webmanifest"));
    const strings: string[] = [];
    const walk = (value: unknown): void => {
      if (typeof value === "string") strings.push(value);
      else if (value && typeof value === "object") Object.values(value).forEach(walk);
    };
    walk(manifest);
    expect(strings.filter(isExternal)).toEqual([]);
    expect(strings).toContain("/icons/icon-192.png");
  });

  it("the files in public/ load nothing from another origin", async () => {
    const paths = await files("public/", /\.(?:js|svg|html|css)$/);
    expect(paths).toContain("public/theme-init.js");
    for (const path of paths) {
      expect(externalLoads(await read(path)), path).toEqual([]);
    }
  });

  it("no style, component or module loads anything from another origin", async () => {
    const paths = (await files("src/", /\.(?:css|vue|ts)$/)).filter(
      (path) => !/\.test\.ts$/.test(path) && !path.startsWith("src/test/"),
    );
    expect(paths).toContain("src/styles/index.css");
    expect(paths).toContain("src/App.vue");
    const offending = [];
    for (const path of paths) {
      offending.push(...externalLoads(await read(path)).map((load) => `${path}: ${load}`));
    }
    expect(offending).toEqual([]);
  });
});

describe("the shell under the CSP", () => {
  /** `script-src 'self'` blocks inline code, so the shell must have none. */
  it("index.html has no inline script and no inline event handler", async () => {
    const html = await read("index.html");
    const doc = new DOMParser().parseFromString(html, "text/html");
    const inline = [...doc.querySelectorAll("script")].filter((script) => !script.hasAttribute("src"));
    expect(inline.map((script) => script.outerHTML)).toEqual([]);
    const handlers = [...doc.querySelectorAll("*")].flatMap((element) =>
      element.getAttributeNames().filter((name) => name.startsWith("on")),
    );
    expect(handlers).toEqual([]);
    expect(html).not.toMatch(/javascript:/i);
  });

  /**
   * The stored palette is painted before the first frame only if the boot
   * script runs before <body> is parsed: a classic script in <head>, neither
   * async, deferred nor a module.
   */
  it("index.html runs the theme script synchronously in <head>", async () => {
    const doc = new DOMParser().parseFromString(await read("index.html"), "text/html");
    const boot = doc.querySelector('head > script[src="/theme-init.js"]');
    expect(boot).not.toBeNull();
    expect(boot?.hasAttribute("async")).toBe(false);
    expect(boot?.hasAttribute("defer")).toBe(false);
    expect(boot?.getAttribute("type") ?? "").toBe("");
  });

  it("the theme script reads the keys the stores write", async () => {
    const boot = await read("public/theme-init.js");
    const stores = (await read("src/stores/theme.ts")) + (await read("src/i18n/index.ts"));
    for (const key of ["bookclub.theme", "bookclub.mode", "bookclub.locale"]) {
      expect(boot).toContain(`"${key}"`);
      expect(stores).toContain(`"${key}"`);
    }
  });
});
