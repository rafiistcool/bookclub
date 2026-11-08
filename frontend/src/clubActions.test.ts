import { describe, expect, it } from "vitest";
import { clubsFor, pickTargets } from "./clubActions";
import type { ClubRole, ClubSummary } from "./types";

const club = (slug: string, role: ClubRole): ClubSummary => ({ slug, name: slug, role, timezone: "UTC", member_count: 2 });

describe("club actions on a book", () => {
  it("lets admins and owners set the pick, and every member nominate", () => {
    const clubs = [club("a", "owner"), club("b", "admin"), club("c", "member")];
    expect(clubsFor("pick", clubs).map((row) => row.slug)).toEqual(["a", "b"]);
    expect(clubsFor("nominate", clubs).map((row) => row.slug)).toEqual(["a", "b", "c"]);
  });

  it("offers nothing without clubs", () => {
    expect(clubsFor("pick", [])).toEqual([]);
    expect(clubsFor("nominate", undefined)).toEqual([]);
  });
});

describe("where a book can still become the pick", () => {
  it("leaves out the clubs whose pick it already is and the clubs without the right", () => {
    const clubs = [club("a", "owner"), club("b", "admin"), club("c", "member")];
    expect(pickTargets(clubs, [{ slug: "a", name: "a" }]).map((row) => row.slug)).toEqual(["b"]);
    expect(pickTargets(clubs, []).map((row) => row.slug)).toEqual(["a", "b"]);
    expect(pickTargets(clubs, [{ slug: "a", name: "a" }, { slug: "b", name: "b" }])).toEqual([]);
    expect(pickTargets(undefined, undefined)).toEqual([]);
  });
});
