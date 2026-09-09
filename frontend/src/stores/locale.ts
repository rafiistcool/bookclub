import { defineStore } from "pinia";
import { api } from "../api/client";
import {
  applyLocale,
  detectLocale,
  isLocale,
  type Locale,
} from "../i18n";
import type { User } from "../types";

export const useLocale = defineStore("locale", {
  state: () => ({
    locale: detectLocale() as Locale,
  }),
  actions: {
    init() {
      applyLocale(this.locale);
    },
    adopt(user: User | null) {
      if (!user || !isLocale(user.locale)) return;
      this.locale = user.locale;
      applyLocale(this.locale);
    },
    setLocale(locale: Locale) {
      if (locale === this.locale) return;
      this.locale = locale;
      applyLocale(locale);
      void this.persist();
    },
    async persist() {
      try {
        await api.savePreferences({ locale: this.locale });
      } catch {
        /* Logged out or offline; localStorage keeps the choice. */
      }
    },
  },
});
