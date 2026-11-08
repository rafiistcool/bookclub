import { describe, expect, it, vi } from "vitest";

/**
 * Rules of the stylesheets that no component test can see (jsdom draws no
 * CSS): the placeholder contrast in every palette, the 44 px hit areas, and
 * the frame's measures. Read from disk, as the timezone test does.
 */

type Fs = { readFileSync(path: URL, encoding: "utf8"): string };

/** The src directory (a variable: Vite rewrites `new URL` of a literal). */
const ROOT = "../";

async function read(path: string): Promise<string> {
  const fs = await vi.importActual<Fs>("node:fs");
  return fs.readFileSync(new URL(ROOT + path, import.meta.url), "utf8");
}

function luminance(hex: string): number {
  const [r, g, b] = [0, 2, 4].map((at) => parseInt(hex.slice(1 + at, 3 + at), 16) / 255);
  const linear = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * linear(r!) + 0.7152 * linear(g!) + 0.0722 * linear(b!);
}

function contrast(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light! + 0.05) / (dark! + 0.05);
}

/** The rule's body for `selector` in a stylesheet or a component's <style>. */
function rule(css: string, selector: string): string {
  const at = css.indexOf(`${selector} {`);
  expect(at, selector).toBeGreaterThanOrEqual(0);
  return css.slice(at, css.indexOf("}", at));
}

describe("stylesheets", () => {
  it("gives every field one placeholder colour that reads 4.5:1 on every surface", async () => {
    const base = await read("styles/base.css");
    const placeholder = rule(base, "::placeholder");
    expect(placeholder).toContain("color: var(--placeholder)");
    // Browsers fade placeholders on their own; the token is the whole colour.
    expect(placeholder).toContain("opacity: 1");
    expect(await read("styles/tokens.css")).toContain("--placeholder: var(--text-subtle);");

    const themes = await read("styles/themes.css");
    const blocks = themes.split(/\n(?=html\[|:root,)/).filter((block) => block.includes("--text-subtle"));
    expect(blocks.length).toBeGreaterThanOrEqual(8);
    for (const block of blocks) {
      const token = (name: string) => new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`).exec(block)?.[1];
      const subtle = token("text-subtle")!;
      for (const surface of ["bg", "surface", "surface-2"]) {
        const ratio = contrast(subtle, token(surface)!);
        expect(ratio, `${block.split("\n")[0]} ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("gives Home's reading titles and the feed heads a hit area a finger high", async () => {
    const home = await read("pages/HomePage.vue");
    expect(rule(home, ".reading-link")).toContain("position: relative");
    expect(rule(home, ".reading-link::before")).toContain("inset: min(0px, calc((100% - var(--tap)) / 2)) 0");
    // Under a one-line title the area reaches the progress button, which is drawn over it.
    expect(rule(home, ".reading-meta :deep(.progress-trigger)")).toContain("position: relative");
    const feed = await read("components/DiaryFeed.vue");
    expect(rule(feed, ".feed-head")).toContain("position: relative");
    expect(rule(feed, ".feed-head::before")).toContain("inset: min(0px, calc((100% - var(--tap)) / 2)) 0");
  });

  it("keeps the book page's status segment a hand wide on a wide panel", async () => {
    const book = await read("pages/BookDetailPage.vue");
    const wide = book.slice(book.indexOf("@media (min-width: 720px) {\n  .status-control"));
    expect(rule(wide, ".status-control")).toContain("max-width: 30rem");
  });

  it("keeps four statuses whole: content-wide segments, narrower padding on a narrow phone", async () => {
    const book = await read("pages/BookDetailPage.vue");
    const segment = rule(book, ".status-control button");
    expect(segment).toContain("flex: 1 1 0");
    expect(segment).toContain("min-width: max-content");
    const components = await read("styles/components.css");
    const narrow = components.slice(components.indexOf("@media (max-width: 389px) {\n  .segmented button"));
    expect(rule(narrow, ".segmented button")).toContain("padding: 0 var(--space-1)");
  });

  it("puts the scan page's four statuses in two rows of two on a 320 px phone", async () => {
    const scan = await read("pages/ScanPage.vue");
    expect(scan).toContain('class="segmented add-to"');
    const narrow = scan.slice(scan.indexOf("@media (max-width: 359px) {\n  .segmented.add-to"));
    const grid = rule(narrow, "  .segmented.add-to");
    expect(grid).toContain("display: grid");
    expect(grid).toContain("grid-template-columns: repeat(2, minmax(0, 1fr))");
    // Each segment still a finger high: components.css gives it --tap.
    expect(rule(await read("styles/components.css"), ".segmented button")).toContain("min-height: var(--tap)");
    // No section of the hit card widens it past the screen.
    expect(rule(scan, ".hit-section")).toContain("min-width: 0");
  });

  it("never cuts a cover tile's year or language", async () => {
    const chooser = await read("components/CoverChooserSheet.vue");
    expect(rule(chooser, ".label-facts")).not.toContain("text-overflow");
    expect(rule(chooser, ".label-facts")).not.toContain("nowrap");
    // A fact moves to the next line whole; only one wider than the tile breaks.
    const fact = rule(chooser, ".label-fact");
    expect(fact).toContain("display: inline-block");
    expect(fact).toContain("max-width: 100%");
  });

  it("gives the diary's reaction chips and picks a hit area a finger high", async () => {
    const components = await read("styles/components.css");
    expect(rule(components, ".chip")).toContain("position: relative");
    // The 40 px pill's ::before reaches 44 px.
    expect(rule(components, ".chip::before")).toContain("inset: min(-2px, calc((100% - var(--tap)) / 2)) -2px");
    const entry = await read("components/DiaryEntry.vue");
    expect(rule(entry, ".chip.reaction")).toContain("min-height: 40px");
    // The picks sit 2 px apart, so each is the full target itself.
    const pick = rule(entry, ".reaction-pick");
    expect(pick).toContain("min-width: var(--tap)");
    expect(pick).toContain("min-height: var(--tap)");
  });

  it("keeps Discover's tick chips one width, ticked or not", async () => {
    const discover = await read("pages/DiscoverPage.vue");
    const px = (value: string) => value.replace("var(--space-3)", "12px").replace(/calc\((\d+)px ([+-]) (\d+)px\)/,
      (_all, a: string, sign: string, b: string) => String(sign === "+" ? +a + +b : +a - +b));
    const off = Number(px(/padding-inline: ([^;]+);/.exec(rule(discover, ".tick-chip"))![1]!));
    const on = Number(px(/padding-inline: ([^;]+);/.exec(rule(discover, ".tick-chip.active"))![1]!));
    const check = rule(discover, ".chip-check");
    const tick = Number(/width: (\d+)px/.exec(check)![1]) + Number(/margin-right: (\d+)px/.exec(check)![1]);
    expect(2 * on + tick).toBe(2 * off);
    // Every tick chip shows its tick only while pressed.
    expect(discover.match(/class="chip[^"]*tick-chip"/g)).toHaveLength(2);
  });

  it("sizes the club page's h1 itself, above the sections' h2s", async () => {
    const club = await read("layouts/ClubLayout.vue");
    const title = rule(club, ".club-title");
    expect(title).toContain("font-size: var(--text-xl)");
    expect(title).not.toContain("font: inherit");
  });

  it("lets sticky bars follow the phone's top bar as it slides away", async () => {
    expect(await read("styles/tokens.css")).toContain("--bar-bottom: calc(var(--header-h) + env(safe-area-inset-top));");
    const shell = await read("layouts/AppShell.vue");
    // Away, only the bar's own height leaves: the safe area stays covered.
    expect(rule(shell, "  .topbar.away")).toContain("translateY(calc(-1 * var(--header-h)))");
    expect(rule(shell, "  .shell.bar-away")).toContain("--bar-bottom: env(safe-area-inset-top)");
    expect(shell).toContain("@media (max-width: 720px)");
    // Its words fade with it instead of sitting under the status bar.
    const away = rule(shell, "  .topbar.away .wordmark");
    expect(away).toContain("opacity: 0");
    expect(away).toContain("pointer-events: none");
    for (const path of ["layouts/ClubLayout.vue", "pages/DiscoverPage.vue"]) {
      expect(await read(path), path).toContain("top: var(--bar-bottom);");
    }
  });

  it("keeps the longest status whole beside its chevron on a narrow shelf card", async () => {
    const card = await read("components/BookCard.vue");
    // The card is the container the chip measures itself against.
    expect(rule(card, ".shelf-card")).toContain("container-type: inline-size");
    const narrow = card.slice(card.indexOf("@container (max-width: 108px)"));
    expect(card).toContain("@container (max-width: 108px)");
    expect(rule(narrow, "  .status-label")).toContain("font-size: var(--text-xs)");
    expect(rule(narrow, "  .status-chevron")).toContain("width: 14px");
    expect(rule(narrow, "  .shelf-status")).toContain("gap: 2px");
    expect(rule(card, ".status-label")).toContain("hyphens: auto");
  });
});
