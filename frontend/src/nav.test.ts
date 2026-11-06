import { describe, expect, it } from "vitest";
import { isActive, NAV_ITEMS, PROFILE_ITEM } from "./nav";

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

  it("keeps the phone's tab bar at five, the profile last and out of the sidebar's rows", () => {
    expect(NAV_ITEMS.map((item) => item.to)).toEqual(["/", "/discover", "/shelf", "/club", "/me"]);
    expect(NAV_ITEMS.filter((item) => item.bottomBar)).toHaveLength(5);
    expect(NAV_ITEMS.filter((item) => item.sidebar !== false).map((item) => item.to)).toEqual([
      "/",
      "/discover",
      "/shelf",
      "/club",
    ]);
    expect(NAV_ITEMS.at(-1)).toBe(PROFILE_ITEM);
    expect(PROFILE_ITEM).toMatchObject({ labelKey: "nav.profile", icon: "profile", bottomBar: true, sidebar: false });
  });

  it("lights the profile on the pages under it: friends, settings, the instance admin", () => {
    for (const path of ["/me", "/friends", "/settings", "/settings/account", "/admin"]) {
      expect(isActive(path, PROFILE_ITEM)).toBe(true);
    }
    for (const path of ["/media", "/meet", "/friendship", "/u/grace", "/stats", "/shelf"]) {
      expect(isActive(path, PROFILE_ITEM)).toBe(false);
    }
    // The year stays with the shelf.
    expect(isActive("/stats", NAV_ITEMS.find((item) => item.to === "/shelf")!)).toBe(true);
  });

  it("keeps Home to the root and other tabs to their prefix", () => {
    expect(isActive("/", "/")).toBe(true);
    expect(isActive("/discover", "/")).toBe(false);
    expect(isActive("/shelf/physical", "/shelf")).toBe(true);
  });
});
