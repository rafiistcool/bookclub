import { describe, expect, it } from "vitest";
import { loginRoute, safeNextPath } from "./nextPath";

describe("safeNextPath", () => {
  it("keeps a path inside the app", () => {
    expect(safeNextPath("/shelf")).toBe("/shelf");
    expect(safeNextPath("/book/4?x=1#entry-2")).toBe("/book/4?x=1#entry-2");
  });

  it("sends anything that could leave the app to the home page", () => {
    for (const raw of [
      undefined,
      null,
      ["/shelf"],
      "",
      "shelf",
      "https://evil.example/",
      "//evil.example/",
      "/\\evil.example/",
      "/\t/evil.example/",
      "/\n/evil.example/",
    ]) {
      expect(safeNextPath(raw)).toBe("/");
    }
  });

  it("builds the login route that comes back afterwards", () => {
    expect(loginRoute("/discover?q=dune")).toEqual({ path: "/login", query: { next: "/discover?q=dune" } });
    expect(loginRoute("//evil.example")).toEqual({ path: "/login", query: { next: "/" } });
  });
});
