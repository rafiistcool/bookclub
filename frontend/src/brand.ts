import type { ClubConfig } from "./types";

const DEFAULTS: ClubConfig = {
  name: "Bookclub",
  theme: "#b44a2a",
  theme_dark: "#8e361c",
  public_url: "",
  timezone: "UTC",
};

export function applyBrand(config: ClubConfig): void {
  const name = config.name || DEFAULTS.name;
  const theme = config.theme || DEFAULTS.theme;
  const themeDark = config.theme_dark || DEFAULTS.theme_dark;
  document.title = name;
  document.documentElement.style.setProperty("--accent", theme);
  document.documentElement.style.setProperty("--accent-dark", themeDark);
  const themeMeta = document.querySelector('meta[name="theme-color"]');
  if (themeMeta) themeMeta.setAttribute("content", theme);
  const apple = document.querySelector('meta[name="apple-mobile-web-app-title"]');
  if (apple) apple.setAttribute("content", name);
  const manifest = document.querySelector('link[rel="manifest"]');
  if (manifest) {
    const payload = {
      name,
      short_name: name.slice(0, 32),
      start_url: "/",
      display: "standalone",
      background_color: "#f6f1e8",
      theme_color: theme,
    };
    const blob = new Blob([JSON.stringify(payload)], {
      type: "application/manifest+json",
    });
    manifest.setAttribute("href", URL.createObjectURL(blob));
  }
}
