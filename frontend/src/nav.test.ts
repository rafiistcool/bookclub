import { describe, expect, it } from "vitest";
import { isActive, NAV_ITEMS } from "./nav";

const club = NAV_ITEMS.find((item) => item.to === "/club")!;

describe("navigation", () => {
  it("lights the Club tab on every club page", () => {
    expect(isActive("/club", club)).toBe(true);
    expect(isActive("/c/readers-ab12", club)).toBe(true);
    expect(isActive("/c/readers-ab12/vote", club)).toBe(true);
    expect(isActive("/clubs/new", club)).toBe(true);
    expect(isActive("/clubby", club)).toBe(false);
    expect(isActive("/discover", club)).toBe(false);
  });

  it("keeps Home to the root and other tabs to their prefix", () => {
    expect(isActive("/", "/")).toBe(true);
    expect(isActive("/discover", "/")).toBe(false);
    expect(isActive("/shelf/physical", "/shelf")).toBe(true);
  });
});
