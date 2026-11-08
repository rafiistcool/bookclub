import { describe, expect, it } from "vitest";
import { backTarget } from "./backTarget";

describe("backTarget", () => {
  it("falls back to Discover when the book was opened directly", () => {
    expect(backTarget(null)).toEqual({ kind: "fallback", to: "/discover" });
    expect(backTarget(undefined)).toEqual({ kind: "fallback", to: "/discover" });
    expect(backTarget("")).toEqual({ kind: "fallback", to: "/discover" });
  });

  it("names the page the reader came from", () => {
    expect(backTarget("/")).toEqual({ kind: "back", page: "home" });
    expect(backTarget("/shelf")).toEqual({ kind: "back", page: "shelf" });
    expect(backTarget("/shelf/physical")).toEqual({ kind: "back", page: "shelf" });
    expect(backTarget("/shelf?status=finished")).toEqual({ kind: "back", page: "shelf" });
    expect(backTarget("/discover?q=stoner#top")).toEqual({ kind: "back", page: "discover" });
    expect(backTarget("/scan")).toEqual({ kind: "back", page: "discover" });
    expect(backTarget("/c/lesekreis-suedstadt-gqjf/discussions")).toEqual({ kind: "back", page: "club" });
    expect(backTarget("/clubs")).toEqual({ kind: "back", page: "club" });
    expect(backTarget("/me")).toEqual({ kind: "back", page: "profile" });
  });

  it("names the member whose shelf or profile it was", () => {
    expect(backTarget("/u/jonas/shelf")).toEqual({ kind: "back", page: "user", username: "jonas" });
    expect(backTarget("/u/lea")).toEqual({ kind: "back", page: "user", username: "lea" });
    expect(backTarget("/u/%E0%A4%A")).toEqual({ kind: "back", page: "user", username: "%E0%A4%A" });
  });

  it("goes back without a name from any other page", () => {
    expect(backTarget("/book/162")).toEqual({ kind: "back", page: null });
    expect(backTarget("/diary")).toEqual({ kind: "back", page: null });
    expect(backTarget("/stats")).toEqual({ kind: "back", page: null });
  });
});
