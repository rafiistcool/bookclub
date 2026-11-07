import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/client", () => ({
  api: { config: vi.fn(), savePreferences: vi.fn() },
}));

import { memoryStorage } from "../test/storage";
import { useService } from "./service";
import { useTheme } from "./theme";

const root = document.documentElement;

/** A matchMedia stand-in whose OS preference the test can flip. */
function fakeSystem(dark: boolean) {
  const listeners: ((event: { matches: boolean }) => void)[] = [];
  const media = {
    get matches() {
      return dark;
    },
    addEventListener: (_: string, fn: (event: { matches: boolean }) => void) => listeners.push(fn),
    removeEventListener() {},
  };
  vi.stubGlobal("matchMedia", () => media);
  return {
    set(next: boolean) {
      dark = next;
      for (const fn of listeners) fn({ matches: next });
    },
  };
}

describe("theme store and the service accent", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    setActivePinia(createPinia());
    useService().$patch({ theme: "#b44a2a", theme_dark: "#8e361c" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    root.removeAttribute("data-theme");
    root.removeAttribute("data-mode");
    root.removeAttribute("style");
  });

  it("puts the service accent on Paper light", () => {
    fakeSystem(false);
    localStorage.setItem("bookclub.mode", "light");
    useTheme().init();
    expect(root.style.getPropertyValue("--accent")).toBe("#b44a2a");
    expect(root.style.getPropertyValue("--accent-dark")).toBe("#8e361c");
  });

  it("leaves Paper dark its own accent, so the dark ink stays readable", () => {
    fakeSystem(false);
    const theme = useTheme();
    localStorage.setItem("bookclub.mode", "light");
    theme.init();
    theme.setMode("dark");
    expect(root.getAttribute("data-mode")).toBe("dark");
    expect(root.style.getPropertyValue("--accent")).toBe("");
    expect(root.style.getPropertyValue("--accent-dark")).toBe("");
  });

  it("never overrides the accent of the other palettes, in either mode", () => {
    fakeSystem(false);
    const theme = useTheme();
    theme.init();
    for (const id of ["slate", "forest", "ink"] as const) {
      for (const mode of ["light", "dark"] as const) {
        theme.setTheme(id);
        theme.setMode(mode);
        expect(root.style.getPropertyValue("--accent"), `${id} ${mode}`).toBe("");
      }
    }
  });

  it("follows a live OS switch in system mode", () => {
    const system = fakeSystem(false);
    const theme = useTheme();
    theme.init();
    expect(theme.mode).toBe("system");
    expect(root.getAttribute("data-mode")).toBe("light");
    expect(root.style.getPropertyValue("--accent")).toBe("#b44a2a");

    system.set(true);
    expect(theme.resolvedMode).toBe("dark");
    expect(root.getAttribute("data-mode")).toBe("dark");
    expect(root.style.getPropertyValue("--accent")).toBe("");

    system.set(false);
    expect(root.getAttribute("data-mode")).toBe("light");
    expect(root.style.getPropertyValue("--accent")).toBe("#b44a2a");
  });

  it("skips a service accent the palette's ink cannot be read on", () => {
    fakeSystem(false);
    localStorage.setItem("bookclub.mode", "light");
    // What themes.css gives Paper light; jsdom loads no stylesheet.
    root.style.setProperty("--accent-ink", "#fffaf6");
    useService().$patch({ theme: "#f0d060" });
    useTheme().init();
    expect(root.style.getPropertyValue("--accent")).toBe("");
  });
});
