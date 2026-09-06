import { defineStore } from "pinia";
import { api } from "../api/client";
import { applyClubAccent, applyThemeColor } from "../brand";
import type { ColorMode, ThemeId, User } from "../types";
import { useClub } from "./club";

export const THEMES = [
  {
    id: "paper",
    label: "Paper",
    blurb: "Warm ivory and brick. The house style.",
  },
  {
    id: "slate",
    label: "Slate",
    blurb: "Cool greys with a printer's blue.",
  },
  {
    id: "forest",
    label: "Forest",
    blurb: "Soft greens and deep evergreen.",
  },
  {
    id: "ink",
    label: "Ink",
    blurb: "Black on white, maximum contrast.",
  },
] as const satisfies readonly { id: ThemeId; label: string; blurb: string }[];

export const MODES = [
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
  { id: "system", label: "System" },
] as const satisfies readonly { id: ColorMode; label: string }[];

const THEME_KEY = "bookclub.theme";
const MODE_KEY = "bookclub.mode";
const DEFAULT_THEME: ThemeId = "paper";
const DEFAULT_MODE: ColorMode = "system";

const THEME_IDS = THEMES.map((theme) => theme.id) as readonly string[];
const MODE_IDS = MODES.map((mode) => mode.id) as readonly string[];

function isTheme(value: unknown): value is ThemeId {
  return typeof value === "string" && THEME_IDS.includes(value);
}

function isMode(value: unknown): value is ColorMode {
  return typeof value === "string" && MODE_IDS.includes(value);
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* Private mode or a full quota. The server copy is the real one. */
  }
}

function prefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export const useTheme = defineStore("theme", {
  state: () => ({
    theme: DEFAULT_THEME as ThemeId,
    mode: DEFAULT_MODE as ColorMode,
    watching: false,
  }),
  getters: {
    /** The mode actually painted: "system" resolved against the OS. */
    resolvedMode(state): "light" | "dark" {
      if (state.mode === "system") return prefersDark() ? "dark" : "light";
      return state.mode;
    },
  },
  actions: {
    /** Adopt whatever the boot script in index.html already painted. */
    init() {
      const theme = read(THEME_KEY);
      const mode = read(MODE_KEY);
      if (isTheme(theme)) this.theme = theme;
      if (isMode(mode)) this.mode = mode;
      this.apply();
      if (this.watching) return;
      this.watching = true;
      window
        .matchMedia("(prefers-color-scheme: dark)")
        .addEventListener("change", () => {
          if (this.mode === "system") this.apply();
        });
    },
    apply() {
      const root = document.documentElement;
      root.setAttribute("data-theme", this.theme);
      root.setAttribute("data-mode", this.resolvedMode);
      write(THEME_KEY, this.theme);
      write(MODE_KEY, this.mode);
      applyClubAccent(useClub().$state, this.theme);
      applyThemeColor();
    },
    /** Take the server's stored preference as the source of truth on login. */
    adopt(user: User | null) {
      if (!user) return;
      if (isTheme(user.theme)) this.theme = user.theme;
      if (isMode(user.color_mode)) this.mode = user.color_mode;
      this.apply();
    },
    setTheme(theme: ThemeId) {
      if (theme === this.theme) return;
      this.theme = theme;
      this.apply();
      void this.persist({ theme });
    },
    setMode(mode: ColorMode) {
      if (mode === this.mode) return;
      this.mode = mode;
      this.apply();
      void this.persist({ color_mode: mode });
    },
    /** Best effort. The local mirror already holds the choice. */
    async persist(body: { theme?: ThemeId; color_mode?: ColorMode }) {
      try {
        await api.savePreferences(body);
      } catch {
        /* Logged out or offline; localStorage keeps the choice. */
      }
    },
  },
});
