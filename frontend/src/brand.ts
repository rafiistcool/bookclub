import type { ServiceBrand, ThemeId } from "./types";

export const BRAND_DEFAULTS: ServiceBrand = {
  name: "Bookclub",
  theme: "#b44a2a",
  theme_dark: "#8e361c",
  public_url: "",
  signup: "invite",
};
const DEFAULTS = BRAND_DEFAULTS;

/** Name, title, manifest, and icons. Colour is applied separately. */
export function applyBrand(config: ServiceBrand): void {
  const name = config.name || DEFAULTS.name;
  document.title = name;
  const apple = document.querySelector('meta[name="apple-mobile-web-app-title"]');
  if (apple) apple.setAttribute("content", name);
  const manifest = document.querySelector('link[rel="manifest"]');
  if (!manifest) return;
  // A blob: manifest has no base URL of its own, so Chrome ignores a
  // relative start_url ("URL is invalid"); give it and the scope absolute.
  const origin = window.location.origin;
  const payload = {
    name,
    short_name: name.slice(0, 32),
    start_url: `${origin}/`,
    scope: `${origin}/`,
    display: "standalone",
    background_color: readToken("--bg") || "#f7f4ec",
    theme_color: config.theme || DEFAULTS.theme,
  };
  const blob = new Blob([JSON.stringify(payload)], {
    type: "application/manifest+json",
  });
  manifest.setAttribute("href", URL.createObjectURL(blob));
}

/** The service's accent, applied inline, wins over every palette block.
    Only Paper light is built around it: the other three palettes define
    their own accent, and every dark palette pairs a light accent with a dark
    --accent-ink, so the operator's (light-mode) colour would put dark text on
    brick there. Those clear it and keep the palette's own pair. A colour that
    the palette's ink cannot be read on (under 4.5:1) is not applied either. */
export function applyClubAccent(
  config: ServiceBrand,
  theme: ThemeId,
  mode: "light" | "dark",
): void {
  const root = document.documentElement;
  const accent = config.theme || DEFAULTS.theme;
  const ink = parseHex(readToken("--accent-ink"));
  const accentRgb = parseHex(accent);
  const unreadable = !!ink && !!accentRgb && contrast(ink, accentRgb) < 4.5;
  if (theme !== "paper" || mode === "dark" || unreadable) {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--accent-dark");
    return;
  }
  root.style.setProperty("--accent", accent);
  root.style.setProperty("--accent-dark", config.theme_dark || DEFAULTS.theme_dark);
}

type Rgb = [number, number, number];

function parseHex(value: string): Rgb | null {
  const match = /^#([0-9a-f]{6})$/i.exec(value.trim());
  if (!match) return null;
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance([r, g, b]: Rgb): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio of two colours. */
function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Match the browser chrome to the palette's page background. */
export function applyThemeColor(): void {
  const meta = document.querySelector('meta[name="theme-color"]');
  const bg = readToken("--bg");
  if (meta && bg) meta.setAttribute("content", bg);
}

function readToken(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
