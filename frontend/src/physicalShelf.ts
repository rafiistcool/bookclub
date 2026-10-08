import { orderedShelf } from "./shelf";
import type { ShelfItem } from "./types";

export type ShelfMaterial = "oak" | "walnut" | "white";
export type SpineStyle = { color: string; width: number };
export type PhysicalShelfLayout = {
  rows: number[][];
  material: ShelfMaterial;
  spines: Record<string, SpineStyle>;
};

const MIN_ROWS = 3;
const MAX_ROWS = 12;
const ROW_CAPACITY = 560;
const LIGHT_INK = "#fff8ed";
const DARK_INK = "#2f2b26";
const PALETTE = [
  "#456052", "#6b3544", "#334b66", "#b99345", "#dccdb6", "#ac6552",
  "#506e74", "#7a6851", "#786285", "#c2bda2", "#8e4939", "#466453",
];

function stableHash(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index++) {
    hash = Math.imul(hash ^ value.charCodeAt(index), 16777619);
  }
  return hash >>> 0;
}

function luminance(color: string): number | null {
  const value = color.replace(/^#/, "");
  const hex = /^[\da-f]{3}$/i.test(value)
    ? value.split("").map((digit) => digit + digit).join("")
    : value;
  if (!/^[\da-f]{6}$/i.test(hex)) return null;
  const channel = (offset: number) => {
    const srgb = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

/** Pick the stronger contrast against a custom hex spine color. */
export function spineInk(color: string): string {
  const background = luminance(color);
  if (background == null) return LIGHT_INK;
  const contrast = (ink: string) => {
    const foreground = luminance(ink)!;
    return (Math.max(background, foreground) + 0.05) / (Math.min(background, foreground) + 0.05);
  };
  return contrast(DARK_INK) > contrast(LIGHT_INK) ? DARK_INK : LIGHT_INK;
}

/** Edition-independent fallback when a catalogue does not supply a real spine. */
export function defaultSpine(item: ShelfItem): SpineStyle & { height: number; ink: string; variant: number } {
  const hash = stableHash(item.book.ol_work_key || `${item.book.title}:${item.book.authors}`);
  const color = PALETTE[hash % PALETTE.length]!;
  const pages = item.book.pages;
  const width = typeof pages === "number" && Number.isFinite(pages) && pages > 0
    ? Math.min(64, Math.max(26, Math.round(24 + pages / 18)))
    : 32 + (hash >>> 8) % 17;
  return {
    color,
    width,
    height: 150 + (hash >>> 16) % 41,
    ink: spineInk(color),
    variant: (hash >>> 24) % 4,
  };
}

/** Keep manual placements and append catalogue additions after the last placed book. */
export function reconcileLayout(layout: PhysicalShelfLayout, items: ShelfItem[]): PhysicalShelfLayout {
  const ordered = orderedShelf(items);
  const byId = new Map(ordered.map((item) => [item.id, item]));
  const seen = new Set<number>();
  const rows: number[][] = Array.from({ length: Math.max(MIN_ROWS, Math.min(MAX_ROWS, layout.rows.length)) }, () => []);
  layout.rows.forEach((row, rowIndex) => {
    const target = rows[Math.min(rowIndex, MAX_ROWS - 1)]!;
    row.forEach((id) => {
      if (!byId.has(id) || seen.has(id)) return;
      seen.add(id);
      target.push(id);
    });
  });
  const spines: Record<string, SpineStyle> = {};
  for (const [id, style] of Object.entries(layout.spines)) {
    if (byId.has(Number(id))) spines[id] = { ...style };
  }
  const width = (id: number) => spines[String(id)]?.width ?? defaultSpine(byId.get(id)!).width;
  let rowIndex = rows.reduce((last, row, index) => row.length ? index : last, 0);
  let occupied = rows[rowIndex]!.reduce((total, id) => total + width(id), 0);
  for (const item of ordered) {
    if (seen.has(item.id)) continue;
    const bookWidth = width(item.id);
    if (occupied > 0 && occupied + bookWidth > ROW_CAPACITY && rowIndex < MAX_ROWS - 1) {
      rowIndex++;
      if (!rows[rowIndex]) rows.push([]);
      occupied = 0;
    }
    rows[rowIndex]!.push(item.id);
    occupied += bookWidth;
    seen.add(item.id);
  }
  return { rows, material: layout.material, spines };
}

/** The insertion index refers to the destination after removing the moving book. */
export function moveBook(layout: PhysicalShelfLayout, id: number, targetRow: number, targetIndex: number): PhysicalShelfLayout {
  if (!Number.isInteger(targetRow) || targetRow < 0 || targetRow >= layout.rows.length
    || !Number.isFinite(targetIndex) || !layout.rows.some((row) => row.includes(id))) return layout;
  const seen = new Set<number>();
  const rows = layout.rows.map((row) => row.filter((bookId) => {
    if (bookId === id || seen.has(bookId)) return false;
    seen.add(bookId);
    return true;
  }));
  const destination = rows[targetRow]!;
  destination.splice(Math.max(0, Math.min(destination.length, Math.trunc(targetIndex))), 0, id);
  return {
    ...layout,
    rows,
    spines: Object.fromEntries(Object.entries(layout.spines).map(([key, style]) => [key, { ...style }])),
  };
}
