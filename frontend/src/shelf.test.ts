import { describe, expect, it } from "vitest";
import { clubDate, orderedShelf, readingMonth, readingMonthLabel, shelfDropPosition, visibleShelf } from "./shelf";
import type { ShelfItem } from "./types";
const item = (id: number, status: ShelfItem["status"], position = 0, shelf_position?: number): ShelfItem => ({
  id, position, shelf_position, status, updated_at: "", rating: null,
  take: "", dnf_reason: "", progress: null,
  book: { id, title: `Book ${id}`, authors: "", cover_id: null, cover_url: null, year: null, ol_work_key: `/works/OL${id}W` },
});

describe("shelf dates and order", () => {
  it("uses the club timezone for offsetless SQLite timestamps", () => {
    expect(clubDate("2025-12-31T23:30:00", "Europe/Berlin")).toBe("2026-01-01");
    expect(clubDate("2026-07-01T01:00:00Z", "America/New_York")).toBe("2026-06-30");
    expect(clubDate(null, "UTC")).toBe("");
  });
  it("initially places unread books above completed books and preserves within-status positions", () => {
    const rows = [item(1, "finished"), item(2, "want_to_read", 1), item(3, "currently_reading"), item(4, "want_to_read")];
    expect(orderedShelf(rows).map(row => row.id)).toEqual([4, 2, 3, 1]);
  });
  it("honors a manual order across statuses, with newly added books first", () => {
    const rows = [item(1, "finished", 0, 0), item(2, "want_to_read", 0, 1), item(3, "want_to_read")];
    expect(orderedShelf(rows).map(row => row.id)).toEqual([3, 1, 2]);
  });
  it("maps filtered drops to the full shelf while preserving hidden books' relative order", () => {
    const a = item(1, "want_to_read", 0, 0), b = item(2, "finished", 0, 1);
    const c = item(3, "want_to_read", 1, 2), d = item(4, "finished", 1, 3);
    expect(shelfDropPosition([a, b, c, d], [c, a], c)).toBe(0);
    expect(shelfDropPosition([a, b, c, d], [c, a], a)).toBe(2);
  });
  it("sorts the finished filter by month across years while retaining manual order within a month", () => {
    const oldest = { ...item(1, "finished", 0, 0), finished_at: "2025-12-20T12:00:00Z" };
    const early = { ...item(2, "finished", 0, 1), finished_at: "2026-01-03T12:00:00Z" };
    const late = { ...item(3, "finished", 0, 2), finished_at: "2026-01-28T12:00:00Z" };
    const unknown = item(4, "finished", 0, 3);
    const rows = [oldest, early, late, unknown];
    expect(visibleShelf(rows, "finished", "Europe/Berlin").map(row => row.id)).toEqual([2, 3, 1, 4]);
    expect(visibleShelf(rows, "all", "Europe/Berlin").map(row => row.id)).toEqual([1, 2, 3, 4]);
  });
  it("labels completion months in the selected language and club timezone", () => {
    const book = { ...item(1, "finished"), finished_at: "2025-12-31T23:30:00" };
    expect(readingMonth(book, "Europe/Berlin")).toBe("2026-01");
    expect(readingMonthLabel("2026-01", "de")).toBe("Januar 2026");
    expect(readingMonthLabel("2026-01", "en")).toBe("January 2026");
    expect(readingMonth({ ...book, finished_at: "invalid" }, "UTC")).toBe("");
    expect(readingMonth({ ...book, status: "currently_reading" }, "UTC")).toBe("");
  });

});
