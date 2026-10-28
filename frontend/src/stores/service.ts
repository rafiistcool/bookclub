import { defineStore } from "pinia";
import { api } from "../api/client";
import { applyBrand, BRAND_DEFAULTS } from "../brand";
import type { ServiceBrand } from "../types";
import { useTheme } from "./theme";

/** The service's name and accent (`GET /api/config`), for the wordmark and the Paper theme. */
export const useService = defineStore("service", {
  state: (): ServiceBrand & { loaded: boolean } => ({ ...BRAND_DEFAULTS, loaded: false }),
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
      // Re-paint so the palette picks up the accent that just arrived.
      useTheme().apply();
    },
  },
});
