import { defineStore } from "pinia";
import { api } from "../api/client";
import { applyBrand, BRAND_DEFAULTS } from "../brand";
import type { ServiceConfig } from "../api/generated";
import type { LegalDoc } from "../types";
import { useTheme } from "./theme";

/** The legal part of `GET /api/config`. */
type ServiceLegal = Pick<ServiceConfig, "legal" | "terms_version" | "contact_email">;

/** Until `/api/config` answers, or when it fails: no texts to link, no terms to accept, no contact. */
export const LEGAL_DEFAULTS: ServiceLegal = { legal: [], terms_version: null, contact_email: null };

/**
 * Every legal text (#53), in the order the links list them: the operator's
 * imprint, privacy policy and terms of use, served by `GET /api/legal/{doc}`.
 */
export const LEGAL_DOCS: readonly LegalDoc[] = ["imprint", "privacy", "terms"];

/** The page of a legal text. Each one is public, signed in or out. */
export function legalPath(doc: LegalDoc): string {
  return `/${doc}`;
}

/**
 * The service's name and accent (`GET /api/config`), for the wordmark and the
 * Paper theme, and its legal settings: the documents `/imprint`, `/privacy`
 * and `/terms` show, the terms version a sign-up accepts, and the public
 * contact address.
 */
export const useService = defineStore("service", {
  state: (): ServiceConfig & { loaded: boolean } => ({ ...BRAND_DEFAULTS, ...LEGAL_DEFAULTS, loaded: false }),
  getters: {
    /** Whether the operator provides `doc` (in either language). */
    hasLegal: (state) => (doc: LegalDoc) => state.legal.includes(doc),
  },
  actions: {
    async load() {
      try {
        this.$patch(await api.config());
      } catch {
        this.$patch(BRAND_DEFAULTS);
      } finally {
        this.loaded = true;
      }
      applyBrand(this.$state);
      // Re-paint so the palette picks up the accent that just arrived, but
      // only once the theme store holds the reader's palette. At boot main.ts
      // calls init() after this, and init() paints with this accent. Until
      // then the store holds its defaults: painting them would replace what
      // public/theme-init.js painted and overwrite the stored choice.
      const theme = useTheme();
      if (theme.watching) theme.apply();
    },
    /**
     * Fetches the legal settings again, after the server said the terms
     * changed since the page loaded them, so the next attempt sends the new
     * version. A failure keeps what the store has.
     */
    async reloadLegal() {
      try {
        const config = await api.config();
        this.$patch({
          legal: config.legal,
          terms_version: config.terms_version,
          contact_email: config.contact_email,
        });
      } catch {
        /* The next attempt shows the error again. */
      }
    },
  },
});
