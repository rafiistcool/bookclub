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
    // True when the user picked EN|DE on the auth screens this session.
    // Login persists that instead of snapping to a backfilled server value.
    explicit: false,
  }),
  actions: {
    init() {
      applyLocale(this.locale);
    },
    adopt(user: User | null) {
      if (!user || !isLocale(user.locale)) return;
      this.locale = user.locale;
      this.explicit = false;
      applyLocale(this.locale);
    },
    /** Auth-screen toggle: counts even if this locale is already showing. */
    choose(locale: Locale) {
      this.explicit = true;
      this.setLocale(locale);
    },
    clearExplicit() {
      this.explicit = false;
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
        this.explicit = false;
      } catch {
        /* Logged out or offline; localStorage keeps the choice. */
      }
    },
  },
});
