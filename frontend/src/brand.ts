import type { ClubConfig, ThemeId } from "./types";

const DEFAULTS: ClubConfig = {
  name: "Bookclub",
  theme: "#b44a2a",
  theme_dark: "#8e361c",
  public_url: "",
  timezone: "UTC",
};

/** Name, title, manifest, and icons. Colour is applied separately. */
export function applyBrand(config: ClubConfig): void {
  const name = config.name || DEFAULTS.name;
  document.title = name;
  const apple = document.querySelector('meta[name="apple-mobile-web-app-title"]');
  if (apple) apple.setAttribute("content", name);
  const manifest = document.querySelector('link[rel="manifest"]');
  if (!manifest) return;
  const payload = {
    name,
    short_name: name.slice(0, 32),
    start_url: "/",
    display: "standalone",
    background_color: readToken("--bg") || "#f7f4ec",
    theme_color: config.theme || DEFAULTS.theme,
  };
  const blob = new Blob([JSON.stringify(payload)], {
    type: "application/manifest+json",
  });
  manifest.setAttribute("href", URL.createObjectURL(blob));
}

/** The club's configured accent, applied inline, wins over every palette
    block. Only Paper is built around it — the other three define their own
    accent and would render an off-palette colour, so they clear it. */
export function applyClubAccent(config: ClubConfig, theme: ThemeId): void {
  const root = document.documentElement;
  if (theme !== "paper") {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--accent-dark");
    return;
  }
  root.style.setProperty("--accent", config.theme || DEFAULTS.theme);
  root.style.setProperty("--accent-dark", config.theme_dark || DEFAULTS.theme_dark);
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
