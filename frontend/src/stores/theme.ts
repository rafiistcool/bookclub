import { defineStore } from "pinia";

export type ThemePref = "system" | "light" | "dark";

const KEY = "bookclub.theme";

function readPref(): ThemePref {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    /* private mode */
  }
  return "system";
}

function systemDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: dark)").matches;
}

export const useTheme = defineStore("theme", {
  state: () => ({
    pref: "system" as ThemePref,
    accent: "#b44a2a",
  }),
  getters: {
    resolved(state): "light" | "dark" {
      if (state.pref === "system") return systemDark() ? "dark" : "light";
      return state.pref;
    },
  },
  actions: {
    init() {
      this.pref = readPref();
      this.apply();
      window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener?.("change", () => {
        if (this.pref === "system") this.apply();
      });
    },
    set(pref: ThemePref) {
      this.pref = pref;
      try {
        window.localStorage.setItem(KEY, pref);
      } catch {
        /* ignore */
      }
      this.apply();
    },
    apply() {
      const root = document.documentElement;
      if (this.pref === "system") root.removeAttribute("data-theme");
      else root.setAttribute("data-theme", this.pref);
      const meta = document.querySelector('meta[name="theme-color"]');
      if (meta) meta.setAttribute("content", this.resolved === "dark" ? "#191511" : this.accent);
    },
  },
});
