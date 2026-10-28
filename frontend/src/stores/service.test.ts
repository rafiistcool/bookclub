import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const config = vi.fn();

vi.mock("../api/client", () => ({
  api: {
    config: (...args: unknown[]) => config(...args),
    savePreferences: vi.fn(),
  },
}));

import { memoryStorage } from "../test/storage";
import { useService } from "./service";
import { useTheme } from "./theme";

const root = document.documentElement;

describe("service config at boot", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
    setActivePinia(createPinia());
    config.mockReset();
    config.mockResolvedValue({
      name: "Bookclub",
      theme: "#123456",
      theme_dark: "#0a1b2c",
      public_url: "https://bookclub.test",
      signup: "invite",
      legal: [],
      terms_version: null,
      contact_email: null,
    });
    // What public/theme-init.js finds in storage and paints before <body>.
    localStorage.setItem("bookclub.theme", "ink");
    localStorage.setItem("bookclub.mode", "dark");
    root.setAttribute("data-theme", "ink");
    root.setAttribute("data-mode", "dark");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    root.removeAttribute("data-theme");
    root.removeAttribute("data-mode");
    root.removeAttribute("style");
  });

  it("keeps the stored palette when the config lands before the theme store's init()", async () => {
    // main.ts: await useService().load(); then useTheme().init().
    await useService().load();
    expect(localStorage.getItem("bookclub.theme")).toBe("ink");
    expect(localStorage.getItem("bookclub.mode")).toBe("dark");
    expect(root.getAttribute("data-theme")).toBe("ink");
    expect(root.getAttribute("data-mode")).toBe("dark");

    const theme = useTheme();
    theme.init();
    expect(theme.theme).toBe("ink");
    expect(theme.mode).toBe("dark");
    expect(root.getAttribute("data-theme")).toBe("ink");
  });

  it("paints Paper with the club accent once init() has run", async () => {
    localStorage.setItem("bookclub.theme", "paper");
    localStorage.setItem("bookclub.mode", "light");
    await useService().load();
    useTheme().init();
    expect(root.getAttribute("data-theme")).toBe("paper");
    expect(root.style.getPropertyValue("--accent")).toBe("#123456");
  });

  it("re-paints with a new accent when the config loads after init()", async () => {
    localStorage.setItem("bookclub.theme", "paper");
    localStorage.setItem("bookclub.mode", "light");
    useTheme().init();
    await useService().load();
    expect(root.style.getPropertyValue("--accent")).toBe("#123456");
    expect(localStorage.getItem("bookclub.theme")).toBe("paper");
    expect(localStorage.getItem("bookclub.mode")).toBe("light");
  });
});
