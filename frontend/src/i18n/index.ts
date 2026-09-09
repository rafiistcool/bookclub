import { createI18n } from "vue-i18n";
import de from "./de.json";
import en from "./en.json";

export const LOCALES = ["en", "de"] as const;
export type Locale = (typeof LOCALES)[number];
export const DEFAULT_LOCALE: Locale = "en";
export const LOCALE_KEY = "bookclub.locale";

export function isLocale(value: unknown): value is Locale {
  return value === "en" || value === "de";
}

export function detectLocale(): Locale {
  try {
    const stored = localStorage.getItem(LOCALE_KEY);
    if (isLocale(stored)) return stored;
  } catch {
    /* Private mode. */
  }
  try {
    if ((navigator.language || "").toLowerCase().startsWith("de")) return "de";
  } catch {
    /* jsdom without navigator.language. */
  }
  return DEFAULT_LOCALE;
}

export const i18n = createI18n({
  legacy: false,
  locale: detectLocale(),
  fallbackLocale: DEFAULT_LOCALE,
  messages: { en, de },
});

export function applyLocale(locale: Locale): void {
  i18n.global.locale.value = locale;
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale;
  }
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    /* Quota or private mode. */
  }
}

export function t(key: string, named?: Record<string, unknown>): string {
  return String(named ? i18n.global.t(key, named) : i18n.global.t(key));
}

export function tp(key: string, n: number, named?: Record<string, unknown>): string {
  return String(i18n.global.t(key, { n, ...named }, n));
}
