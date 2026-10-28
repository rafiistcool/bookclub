import { describe, expect, it } from "vitest";
import { decorationWidth, defaultSpine, faceHeight, moveBook, reconcileLayout, removeShelfRow, shelfFootprint, spineInk, stackSlots, type PhysicalShelfLayout, type ShelfObjectId } from "./physicalShelf";
import { shelfItem } from "./test/fixtures";
import type { ShelfItem } from "./types";

const item = (id: number, position = id, pages?: number | null): ShelfItem =>
  shelfItem({ id, position, book: pages === undefined ? {} : { pages } });
const layout = (rows: ShelfObjectId[][] = []): PhysicalShelfLayout => ({ rows, material: "oak", spines: {} });

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

  it("packs initial books into rows with room for their widths and defaults to three rows", () => {
    const books = Array.from({ length: 20 }, (_, index) => item(index + 1, index, 900));
    const after = reconcileLayout(layout(), books);
    expect(after.rows).toEqual([
      [1, 2, 3, 4, 5, 6, 7, 8],
      [9, 10, 11, 12, 13, 14, 15, 16],
      [17, 18, 19, 20],
    ]);
    expect(reconcileLayout(layout(), []).rows).toEqual([[], [], []]);
    expect(reconcileLayout(layout(), []).decorations).toEqual({});
    const legacy = { material: "oak", spines: {} } as PhysicalShelfLayout;
    expect(reconcileLayout(legacy, []).rows).toEqual([[], [], []]);
  });

  it("preserves shelves reduced to one or two boards", () => {
    expect(reconcileLayout(layout([[]]), []).rows).toEqual([[]]);
    expect(reconcileLayout(layout([[1], []]), [item(1)]).rows).toEqual([[1], []]);
    const reduced = removeShelfRow(layout([[1], [2]]), 1);
    expect(reconcileLayout(reduced, [item(1), item(2)]).rows).toEqual([[1, 2]]);
  });

  it("preserves mixed object order and only keeps placed decoration records", () => {
    const before: PhysicalShelfLayout = {
      ...layout([["decor:plant-1", 1, "decor:missing", 99, "decor:plant-1", "unprefixed"], [2, "decor:frame-2", 1]]),
      decorations: {
        "decor:plant-1": { kind: "plant", color: "#809566" },
        "decor:frame-2": { kind: "frame", color: "#abc" },
        "decor:unused": { kind: "vase", color: "#fff" },
        unprefixed: { kind: "candle", color: "#ddd" },
      },
    };
    const snapshot = JSON.parse(JSON.stringify(before));
    const after = reconcileLayout(before, [item(1), item(2), item(3)]);
    expect(after.rows).toEqual([["decor:plant-1", 1], [2, "decor:frame-2", 3]]);
    expect(after.decorations).toEqual({
      "decor:plant-1": { kind: "plant", color: "#809566" },
      "decor:frame-2": { kind: "frame", color: "#abc" },
    });
    expect(after.decorations!["decor:plant-1"]).not.toBe(before.decorations!["decor:plant-1"]);
    expect(before).toEqual(snapshot);
    expect(reconcileLayout(layout([["decor:missing", 1]]), [item(1)]).rows).toEqual([[1]]);
  });

  it("includes decoration widths when finding room for new books", () => {
    const before: PhysicalShelfLayout = {
      ...layout([[1, "decor:plant-1"]]),
      spines: { 1: { color: "#abc", width: 470 } },
      decorations: { "decor:plant-1": { kind: "plant", color: "#809566" } },
    };
    expect(reconcileLayout(before, [item(1), item(2)]).rows).toEqual([[1, "decor:plant-1"], [2]]);
    expect(["plant", "vase", "candle", "frame"].map((kind) => decorationWidth(kind as "plant" | "vase" | "candle" | "frame"))).toEqual([76, 58, 50, 74]);
  });

  it("uses custom widths when packing additions and lets the final row overflow at twelve rows", () => {
    const books = Array.from({ length: 100 }, (_, index) => item(index + 1, index, 900));
    const after = reconcileLayout(layout(), books);
    expect(after.rows).toHaveLength(12);
    expect(after.rows[11]).toHaveLength(12);
    expect(after.rows.flat()).toEqual(books.map((book) => book.id));
    const custom = { ...layout([[1]]), spines: { 1: { color: "#abc", width: 550 } } };
    expect(reconcileLayout(custom, [item(1), item(2)]).rows).toEqual([[1], [2]]);
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

  it("moves decorations among books without sharing appearance maps", () => {
    const before: PhysicalShelfLayout = {
      ...layout([[1, "decor:vase-1", 2], ["decor:vase-1", 3]]),
      spines: { 1: { color: "#abc", width: 40 } },
      decorations: { "decor:vase-1": { kind: "vase", color: "#ddd" } },
    };
    const after = moveBook(before, "decor:vase-1", 1, 1);
    expect(after.rows).toEqual([[1, 2], [3, "decor:vase-1"]]);
    expect(before.rows).toEqual([[1, "decor:vase-1", 2], ["decor:vase-1", 3]]);
    expect(after.spines).toEqual(before.spines);
    expect(after.spines[1]).not.toBe(before.spines[1]);
    expect(after.decorations).toEqual(before.decorations);
    expect(after.decorations!["decor:vase-1"]).not.toBe(before.decorations!["decor:vase-1"]);
  });

  it("ignores nonexistent books or invalid destination rows without changing the layout", () => {
    const before = layout([[1], [], []]);
    expect(moveBook(before, 1, -1, 0)).toBe(before);
    expect(moveBook(before, 1, 3, 0)).toBe(before);
    expect(moveBook(before, 1, 0.5, 0)).toBe(before);
    expect(moveBook(before, 2, 0, 0)).toBe(before);
    expect(moveBook(before, "decor:missing", 0, 0)).toBe(before);
  });

  it("removes a board and appends all its books and decorations to the previous board", () => {
    const before: PhysicalShelfLayout = {
      ...layout([[1], ["decor:frame-1", 2], [3]]),
      material: "walnut",
      spines: { 2: { color: "#abc", width: 40 } },
      decorations: { "decor:frame-1": { kind: "frame", color: "#ddd" } },
    };
    const snapshot = JSON.parse(JSON.stringify(before));
    const after = removeShelfRow(before, 1);
    expect(after.rows).toEqual([[1, "decor:frame-1", 2], [3]]);
    expect(after.material).toBe("walnut");
    expect(after.spines).toEqual(before.spines);
    expect(after.spines[2]).not.toBe(before.spines[2]);
    expect(after.decorations).toEqual(before.decorations);
    expect(after.decorations!["decor:frame-1"]).not.toBe(before.decorations!["decor:frame-1"]);
    expect(before).toEqual(snapshot);
  });

  it("keeps contents of the first board on the next board and lets an empty board be removed", () => {
    expect(removeShelfRow(layout([[1, "decor:candle-1"], [2], [3]]), 0).rows).toEqual([[2, 1, "decor:candle-1"], [3]]);
    expect(removeShelfRow(layout([[1], [], [2]]), 1).rows).toEqual([[1], [2]]);
    expect(removeShelfRow(layout([[1], [2], [3]]), 2).rows).toEqual([[1], [2, 3]]);
  });

  it("ignores invalid board indices and keeps the final board", () => {
    const before = layout([[1], [2]]);
    for (const index of [-1, 2, 0.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(removeShelfRow(before, index)).toBe(before);
    }
    const last = layout([[1]]);
    expect(removeShelfRow(last, 0)).toBe(last);
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

  it("prefers the colour sampled from the real cover over the palette", () => {
    const plain = item(1);
    const sampled = { ...plain, book: { ...plain.book, spine_color: "#123456" } };
    expect(defaultSpine(sampled)).toMatchObject({ color: "#123456", ink: spineInk("#123456") });
    expect(defaultSpine(sampled).width).toBe(defaultSpine(plain).width);
    expect(defaultSpine({ ...plain, book: { ...plain.book, spine_color: null } }).color).toBe(defaultSpine(plain).color);
  });

  it("chooses light ink on dark colors and dark ink on light colors, including shorthand hex", () => {
    expect(spineInk("#101010")).toBe("#fff8ed");
    expect(spineInk("#fafafa")).toBe("#2f2b26");
    expect(spineInk("#000")).toBe(spineInk("#000000"));
    expect(spineInk("#FFF")).toBe(spineInk("#ffffff"));
  });
});

describe("book placements", () => {
  it("takes the room each placement needs along the board", () => {
    const book = item(1, 0, 360);
    const spine = defaultSpine(book);
    expect(shelfFootprint(book)).toBe(44);
    expect(shelfFootprint(book, { width: 30 })).toBe(30);
    expect(shelfFootprint(book, { display: "flat", width: 30 })).toBe(spine.height);
    expect(shelfFootprint(book, { display: "cover" })).toBe(Math.round(faceHeight(book) * 2 / 3));
    expect(faceHeight(book)).toBeLessThanOrEqual(spine.height);
  });

  it("packs new books around lying and face-out ones by their real footprint", () => {
    const books = [item(1), item(2), item(3)];
    const wide = { ...layout([[1], [], []]), spines: { "1": { display: "flat" as const } } };
    const occupied = shelfFootprint(books[0]!, wide.spines["1"]);
    expect(occupied).toBeGreaterThan(defaultSpine(books[0]!).width);
    const result = reconcileLayout(wide, books);
    expect(result.rows.flat().sort()).toEqual([1, 2, 3]);
    expect(result.spines).toEqual({ "1": { display: "flat" } });
  });

  it("carries the string lights through reconciliation", () => {
    expect(reconcileLayout({ ...layout([[], [], []]), lights: true }, []).lights).toBe(true);
    expect(reconcileLayout(layout([[], [], []]), []).lights).toBe(false);
  });

  it("piles neighbouring lying books and starts a new pile when one gets too high", () => {
    const lying = (id: number) => id >= 10 ? null : { length: 150 + id * 5, thickness: 40 };
    const slots = stackSlots([1, 2, 10, 3, "decor:a", 4, 5, 6, 7], lying);
    // 1 and 2 share a pile as long as the longer book.
    expect(slots.get(1)).toEqual({ lift: 0, span: 160, level: 0 });
    expect(slots.get(2)).toEqual({ lift: 40, span: 160, level: 1 });
    // An upright book or a decoration ends the pile.
    expect(slots.get(3)).toEqual({ lift: 0, span: 165, level: 0 });
    expect(slots.has(10)).toBe(false);
    // Four 40px books would be 160px high: the fourth starts its own pile.
    expect([4, 5, 6, 7].map((id) => slots.get(id)!.level)).toEqual([0, 1, 2, 0]);
    expect(slots.get(6)).toEqual({ lift: 80, span: 180, level: 2 });
    expect(slots.get(7)).toEqual({ lift: 0, span: 185, level: 0 });
  });

  it("never piles more than five books", () => {
    const slots = stackSlots([1, 2, 3, 4, 5, 6], () => ({ length: 150, thickness: 26 }));
    expect([1, 2, 3, 4, 5, 6].map((id) => slots.get(id)!.level)).toEqual([0, 1, 2, 3, 4, 0]);
  });
});
