import { defineStore } from "pinia";
import { api } from "../api/client";
import { applyBrand } from "../brand";
import type { ClubConfig } from "../types";

const FALLBACK: ClubConfig = {
  name: "Bookclub",
  theme: "#b44a2a",
  theme_dark: "#8e361c",
  public_url: "",
  timezone: "UTC",
};

export const useClub = defineStore("club", {
  state: (): ClubConfig => ({ ...FALLBACK }),
  actions: {
    async load() {
      try {
        const config = await api.config();
        this.$patch(config);
      } catch {
        this.$patch(FALLBACK);
      }
      applyBrand(this.$state);
    },
  },
});
