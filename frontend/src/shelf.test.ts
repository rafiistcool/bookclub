import { describe, expect, it } from "vitest";
import { clubDate, dropPosition, shelfGroups } from "./shelf";
import type { ShelfItem } from "./types";

const item = (id: number, position: number, finished_at: string | null): ShelfItem => ({
  id, position, status: "finished", finished_at, updated_at: "", rating: null,
  take: "", dnf_reason: "", progress: null,
  book: { id, title: `Book ${id}`, authors: "", cover_id: null, cover_url: null, year: null, ol_work_key: `/works/OL${id}W` },
});

describe("shelf month grouping", () => {
  it("uses the club timezone for offsetless SQLite timestamps, including year boundaries", () => {
    expect(clubDate("2025-12-31T23:30:00", "Europe/Berlin")).toBe("2026-01-01");
    expect(clubDate("2026-07-01T01:00:00Z", "America/New_York")).toBe("2026-06-30");
    expect(clubDate(null, "UTC")).toBe("");
  });
  it("orders months newest first, unknown dates last and preserves manual order inside a month", () => {
    const groups = shelfGroups([
      item(1, 0, "2026-07-20T00:00:00Z"), item(2, 1, null),
      item(3, 2, "2026-08-01T00:00:00Z"), item(4, 3, "2026-07-25T00:00:00Z"),
    ], "UTC").filter((group) => group.status === "finished");
    expect(groups.map((group) => group.month)).toEqual(["2026-08", "2026-07", ""]);
    expect(groups[1].items.map((row) => row.id)).toEqual([1, 4]);
  });
  it("maps month-local drops onto the global status order, even when other months are interleaved", () => {
    const a = item(1, 0, null), b = item(2, 1, null), c = item(3, 2, null), d = item(4, 3, null);
    expect(dropPosition([a, b, c, d], [d, b], d, "finished")).toBe(1);
    expect(dropPosition([a, b, c, d], [d, b], b, "finished")).toBe(3);
  });
});
