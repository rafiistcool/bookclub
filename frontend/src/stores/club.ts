import { defineStore } from "pinia";
import { api } from "../api/client";
import { applyBrand } from "../brand";
import type { ClubConfig } from "../types";
import { useTheme } from "./theme";

const FALLBACK: ClubConfig = {
  name: "Bookclub",
  theme: "#b44a2a",
  theme_dark: "#8e361c",
  public_url: "",
  timezone: "UTC",
};

export const useClub = defineStore("club", {
  state: (): ClubConfig & { loaded: boolean } => ({ ...FALLBACK, loaded: false }),
  actions: {
    /** Non-blocking: the app mounts with defaults and re-brands when config arrives. */
    async load() {
      try {
        const config = await api.config();
        this.$patch(config);
      } catch {
        this.$patch(FALLBACK);
      } finally {
        this.loaded = true;
      }
      applyBrand(this.$state);
      const theme = useTheme();
      theme.accent = this.theme || FALLBACK.theme;
      theme.apply();
    },
  },
});
