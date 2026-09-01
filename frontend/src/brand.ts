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
  const root = document.documentElement;
  root.style.setProperty("--accent", theme);
  root.style.setProperty("--accent-dark", themeDark);
  const apple = document.querySelector('meta[name="apple-mobile-web-app-title"]');
  if (apple) apple.setAttribute("content", name);
  if (document.title === "Bookclub" || !document.title) document.title = name;
}
