import { describe, expect, it } from "vitest";
import { clubDate, droppedMonth, finishedDropPosition, flowMarkers, orderedShelf, readingMonth, readingMonthLabel, readingMonthShort, shelfDropPosition, visibleShelf } from "./shelf";
import { shelfItem } from "./test/fixtures";
import type { ShelfItem } from "./types";
const item = (id: number, status: ShelfItem["status"], position = 0, shelf_position?: number): ShelfItem =>
  shelfItem({ id, position, shelf_position: shelf_position ?? null, status });

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

  it("writes a month marker short enough for a cover's width", () => {
    expect(readingMonthShort("2026-09", "de")).toBe("Sept. 2026");
    expect(readingMonthShort("2026-05", "de")).toBe("Mai 2026");
    expect(readingMonthShort("2026-09", "en")).toBe("Sep 2026");
    expect(readingMonthShort("", "de")).toBe("");
  });
});

describe("the finished books as one flow", () => {
  const at = (id: number, iso: string | null, shelf_position?: number): ShelfItem =>
    ({ ...item(id, "finished", 0, shelf_position), finished_at: iso });

  it("orders newest month first, marks each month's first book, and puts undated books last", () => {
    const rows = [at(1, null, 0), at(2, "2026-08-02T12:00:00Z", 1), at(3, "2026-09-20T12:00:00Z", 2),
      at(4, "2026-09-01T12:00:00Z", 3), item(5, "currently_reading")];
    const flow = visibleShelf(rows.filter(row => row.status === "finished"), "finished", "UTC");
    const months = flowMarkers(flow, "UTC");
    expect(flow.map((row, index) => [row.id, months[index]])).toEqual([
      [3, "2026-09"], [4, null], [2, "2026-08"], [1, ""],
    ]);
    expect(flowMarkers([], "UTC")).toEqual([]);
  });

  it("marks a dragged book with the month it would land in, and its old month's next book", () => {
    const octA = at(1, "2026-10-20T12:00:00Z"), octB = at(2, "2026-10-02T12:00:00Z");
    const sepC = at(3, "2026-09-20T12:00:00Z"), sepD = at(4, "2026-09-02T12:00:00Z");
    // October's first book hovers between the two September books.
    expect(flowMarkers([octB, sepC, octA, sepD], "UTC", octA)).toEqual(["2026-10", "2026-09", null, null]);
    // Still beside its own month: nothing changes.
    expect(flowMarkers([octB, octA, sepC, sepD], "UTC", octA)).toEqual(["2026-10", null, "2026-09", null]);
  });

  it("places a drop among the books of the month it lands in, not before the next month's first", () => {
    // September's book was placed first by hand (its date set later), so
    // October's books follow it in the manual order but come first in the grid.
    const sep = at(1, "2026-09-12T12:00:00Z", 0);
    const x = at(2, "2026-10-20T12:00:00Z", 1), a1 = at(3, "2026-10-10T12:00:00Z", 2), a2 = at(4, "2026-10-05T12:00:00Z", 3);
    const all = [sep, x, a1, a2];
    expect(visibleShelf(all, "finished", "UTC").map(row => row.id)).toEqual([2, 3, 4, 1]);
    // x dragged to the end of October, before September's book.
    const dropped = [a1, a2, x, sep];
    expect(droppedMonth(dropped, x, "UTC")).toBe("2026-10");
    expect(shelfDropPosition(all, dropped, x)).toBe(0); // the old anchor: September's book
    const position = finishedDropPosition(all, dropped, x, "2026-10", "UTC");
    expect(position).toBe(3);
    const placed = orderedShelf(all).filter(row => row.id !== x.id);
    placed.splice(position, 0, x);
    const reordered = placed.map((row, index) => ({ ...row, shelf_position: index }));
    expect(visibleShelf(reordered, "finished", "UTC").map(row => row.id)).toEqual([3, 4, 2, 1]);
    // Into September at the boundary, after its last book: last in September.
    const late = [a1, a2, sep, x];
    expect(droppedMonth(late, x, "UTC")).toBe("2026-09");
    expect(finishedDropPosition(all, late, x, "2026-09", "UTC")).toBe(1);
  });

  it("gives a dropped book the month of its neighbour unless it stays beside its own month", () => {
    const sepA = at(1, "2026-09-20T12:00:00Z"), sepB = at(2, "2026-09-02T12:00:00Z");
    const aug = at(3, "2026-08-10T12:00:00Z"), undated = at(4, null);
    // Between two September books, August's book becomes September's.
    expect(droppedMonth([sepB, aug, sepA], aug, "UTC")).toBe("2026-09");
    // Between two other months: the month of the book before it.
    expect(droppedMonth([aug, sepA, undated], sepA, "UTC")).toBe("2026-08");
    // First in the grid: the month after it.
    expect(droppedMonth([aug, sepA, sepB], aug, "UTC")).toBe("2026-09");
    // Beside a book of its own month: it keeps its month.
    expect(droppedMonth([sepA, aug, sepB], aug, "UTC")).toBe("2026-09");
    expect(droppedMonth([sepB, sepA, aug], sepA, "UTC")).toBe("2026-09");
    // A drop never takes a date away: among undated books it keeps its month.
    expect(droppedMonth([undated, sepA], sepA, "UTC")).toBe("2026-09");
    expect(droppedMonth([undated, sepA, undated], sepA, "UTC")).toBe("2026-09");
    expect(droppedMonth([aug, sepA, undated], sepA, "UTC")).toBe("2026-08");
    // An undated book dropped among dated ones takes their month; among undated ones it stays undated.
    expect(droppedMonth([sepA, undated, sepB], undated, "UTC")).toBe("2026-09");
    expect(droppedMonth([undated, at(5, null)], undated, "UTC")).toBe("");
    // Alone it keeps its month.
    expect(droppedMonth([sepA], sepA, "UTC")).toBe("2026-09");
  });
});
