import { describe, expect, it } from "vitest";
import { defaultSpine, moveBook, reconcileLayout, spineInk, type PhysicalShelfLayout } from "./physicalShelf";
import type { ShelfItem } from "./types";

const item = (id: number, position = id, pages?: number | null): ShelfItem => ({
  id, position, status: "want_to_read", updated_at: "", rating: null,
  take: "", dnf_reason: "", progress: null,
  book: {
    id, title: `Book ${id}`, authors: "Author", cover_id: null, cover_url: null,
    year: null, ol_work_key: `/works/OL${id}W`, ...pages === undefined ? {} : { pages },
  },
});
const layout = (rows: number[][] = []): PhysicalShelfLayout => ({ rows, material: "oak", spines: {} });

describe("physical shelf layout", () => {
  it("prunes deleted books and duplicates while preserving placements and adding new books in shelf order", () => {
    const before = { ...layout([[3, 99, 1, 3], [], [2, 1]]), spines: { 1: { color: "#abc", width: 40 }, 99: { color: "#fff", width: 30 } } };
    const snapshot = JSON.parse(JSON.stringify(before));
    const after = reconcileLayout(before, [item(5, 2), item(4, 1), item(3), item(1), item(2)]);
    expect(after.rows).toEqual([[3, 1], [], [2, 4, 5]]);
    expect(after.spines).toEqual({ 1: { color: "#abc", width: 40 } });
    expect(before).toEqual(snapshot);
    expect(after.spines[1]).not.toBe(before.spines[1]);
  });

  it("packs initial books into rows with room for their widths and keeps at least three rows", () => {
    const books = Array.from({ length: 20 }, (_, index) => item(index + 1, index, 900));
    const after = reconcileLayout(layout(), books);
    expect(after.rows).toEqual([
      [1, 2, 3, 4, 5, 6, 7, 8],
      [9, 10, 11, 12, 13, 14, 15, 16],
      [17, 18, 19, 20],
    ]);
    expect(reconcileLayout(layout(), []).rows).toEqual([[], [], []]);
  });

  it("uses custom widths when packing additions and lets the final row overflow at twelve rows", () => {
    const books = Array.from({ length: 100 }, (_, index) => item(index + 1, index, 900));
    const after = reconcileLayout(layout(), books);
    expect(after.rows).toHaveLength(12);
    expect(after.rows[11]).toHaveLength(12);
    expect(after.rows.flat()).toEqual(books.map((book) => book.id));
    const custom = { ...layout([[1]]), spines: { 1: { color: "#abc", width: 550 } } };
    expect(reconcileLayout(custom, [item(1), item(2)]).rows).toEqual([[1], [2], []]);
  });

  it("preserves overflow placements by folding extra rows into the last permitted row", () => {
    const rows = Array.from({ length: 14 }, (_, index) => [index + 1]);
    const after = reconcileLayout(layout(rows), rows.flat().map((id) => item(id)));
    expect(after.rows).toHaveLength(12);
    expect(after.rows[11]).toEqual([12, 13, 14]);
    expect(after.rows.flat()).toEqual(rows.flat());
  });

  it("moves within a row using the destination index after removal", () => {
    const before = layout([[1, 2, 3, 4], [], []]);
    expect(moveBook(before, 1, 0, 2).rows).toEqual([[2, 3, 1, 4], [], []]);
    expect(moveBook(before, 4, 0, 1).rows).toEqual([[1, 4, 2, 3], [], []]);
    expect(before.rows[0]).toEqual([1, 2, 3, 4]);
  });

  it("moves between rows, removes duplicate placements, and clamps the insertion index", () => {
    const before = layout([[1, 2, 1], [3, 2, 1], []]);
    expect(moveBook(before, 1, 1, 1).rows).toEqual([[2], [3, 1], []]);
    expect(moveBook(before, 1, 2, 100).rows).toEqual([[2], [3], [1]]);
    expect(moveBook(before, 3, 0, -2).rows).toEqual([[3, 1, 2], [], []]);
  });

  it("ignores nonexistent books or invalid destination rows without changing the layout", () => {
    const before = layout([[1], [], []]);
    expect(moveBook(before, 1, -1, 0)).toBe(before);
    expect(moveBook(before, 1, 3, 0)).toBe(before);
    expect(moveBook(before, 1, 0.5, 0)).toBe(before);
    expect(moveBook(before, 2, 0, 0)).toBe(before);
  });
});

describe("generated book spines", () => {
  it("gives a work stable fallback styling independent of shelf ID and metadata changes", () => {
    const first = item(1);
    const second = { ...item(8), book: { ...first.book, title: "Translated title", authors: "New author" } };
    expect(defaultSpine(second)).toEqual(defaultSpine(first));
    const spine = defaultSpine(first);
    expect(spine.width).toBeGreaterThanOrEqual(32);
    expect(spine.width).toBeLessThanOrEqual(48);
    expect(spine.height).toBeGreaterThanOrEqual(150);
    expect(spine.height).toBeLessThanOrEqual(190);
    expect(spine.ink).toBe(spineInk(spine.color));
  });

  it("uses page counts for thickness, with bounds and a fallback for unusable counts", () => {
    expect(defaultSpine(item(1, 0, 10)).width).toBe(26);
    expect(defaultSpine(item(1, 0, 360)).width).toBe(44);
    expect(defaultSpine(item(1, 0, 3000)).width).toBe(64);
    for (const pages of [0, -20, null, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(defaultSpine(item(1, 0, pages)).width).toBe(defaultSpine(item(1)).width);
    }
  });

  it("chooses light ink on dark colors and dark ink on light colors, including shorthand hex", () => {
    expect(spineInk("#101010")).toBe("#fff8ed");
    expect(spineInk("#fafafa")).toBe("#2f2b26");
    expect(spineInk("#000")).toBe(spineInk("#000000"));
    expect(spineInk("#FFF")).toBe(spineInk("#ffffff"));
  });
});
