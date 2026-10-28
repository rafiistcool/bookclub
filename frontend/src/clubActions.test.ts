import { describe, expect, it } from "vitest";
import { clubsFor } from "./clubActions";
import type { ClubSummary } from "./types";

const club = (slug: string, role: string): ClubSummary => ({ slug, name: slug, role, timezone: "UTC", member_count: 2 });

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
