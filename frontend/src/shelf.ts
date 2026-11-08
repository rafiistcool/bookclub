import { STATUSES, type Status } from "./constants";
import type { ShelfItem } from "./types";

// SQLite can serialize UTC timestamps without an offset; browsers must not
// interpret those as the device's local time.
export function clubDate(iso: string | null | undefined, timeZone: string): string {
  if (!iso) return "";
  const date = new Date(/(?:Z|[+-]\d{2}:\d{2})$/.test(iso) ? iso : `${iso}Z`);
  if (Number.isNaN(date.getTime())) return "";
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone, year: "numeric", month: "2-digit", day: "2-digit",
  }).formatToParts(date);
  const part = (type: string) => parts.find((p) => p.type === type)?.value;
  return `${part("year")}-${part("month")}-${part("day")}`;
}

/** Match the API's initial order and keep manual placements across statuses. */
export function orderedShelf(items: ShelfItem[]): ShelfItem[] {
  return [...items].sort((a, b) =>
    Number(a.shelf_position != null) - Number(b.shelf_position != null)
    || (a.shelf_position ?? STATUSES.indexOf(a.status)) - (b.shelf_position ?? STATUSES.indexOf(b.status))
    || a.position - b.position || a.id - b.id);
}

export function readingMonth(item: ShelfItem, timezone: string): string {
  return item.status === "finished" ? clubDate(item.finished_at, timezone).slice(0, 7) : "";
}

export function readingMonthLabel(month: string, locale: string): string {
  if (!month) return "";
  return new Intl.DateTimeFormat(locale, { month: "long", year: "numeric", timeZone: "UTC" })
    .format(new Date(`${month}-01T12:00:00Z`));
}

/** A month marker in the finished grid: "Okt. 2026", short enough for a cover's width. */
export function readingMonthShort(month: string, locale: string): string {
  if (!month) return "";
  return new Intl.DateTimeFormat(locale, { month: "short", year: "numeric", timeZone: "UTC" })
    .format(new Date(`${month}-01T12:00:00Z`));
}

/**
 * The finished books are one continuous grid (`visibleShelf(…, "finished")`:
 * newest month first, the manual order inside a month, undated books last).
 * This is the marker over each card in that order: its month on the first
 * book of a month ("" for the undated books), null on the rest. While a book
 * is dragged (`moving`, already in its new slot), it counts with the month
 * it would land in, so the marks show what the drop will do, not where the
 * book came from.
 */
export function flowMarkers(rows: ShelfItem[], timezone: string, moving?: ShelfItem): (string | null)[] {
  const months = rows.map((row) =>
    moving && row.id === moving.id ? droppedMonth(rows, moving, timezone) : readingMonth(row, timezone));
  return months.map((month, index) => (index > 0 && months[index - 1] === month ? null : month));
}

/**
 * The month a book dropped into the finished grid lands in. A book that
 * stays beside a book of its own month keeps its month; otherwise it takes
 * the month of the book before it, or after it at the very start. A drop
 * never takes a date away: undated neighbours do not count for a dated
 * book, so one dropped among them keeps its month.
 */
export function droppedMonth(rows: ShelfItem[], item: ShelfItem, timezone: string): string {
  const index = rows.findIndex((row) => row.id === item.id);
  const own = readingMonth(item, timezone);
  const near = [rows[index - 1], rows[index + 1]].filter((row): row is ShelfItem => Boolean(row));
  const months = near.map((row) => readingMonth(row, timezone)).filter((month) => month || !own);
  if (!months.length || months.includes(own)) return own;
  return months[0]!;
}

/**
 * The shelf position of a book dropped into the finished grid. Only the
 * books of the month it lands in count as neighbours: inside a month the
 * grid follows the manual order, but a card of the next month (whose manual
 * place may be anywhere, after a date was set by hand) says nothing about it.
 */
export function finishedDropPosition(all: ShelfItem[], visible: ShelfItem[], item: ShelfItem, month: string, timezone: string): number {
  const sameMonth = visible.filter((row) => row.id === item.id || readingMonth(row, timezone) === month);
  return shelfDropPosition(all, sameMonth, item);
}

/** The finished filter is chronological; manual order breaks ties within a month. */
export function visibleShelf(items: ShelfItem[], filter: "all" | Status, timezone: string): ShelfItem[] {
  const rows = orderedShelf(items).filter((item) => filter === "all" || item.status === filter);
  return filter === "finished"
    ? rows.sort((a, b) => readingMonth(b, timezone).localeCompare(readingMonth(a, timezone)))
    : rows;
}

/** Map a filtered drop to the full shelf without rearranging hidden books. */
export function shelfDropPosition(all: ShelfItem[], visible: ShelfItem[], item: ShelfItem): number {
  const siblings = orderedShelf(all).filter((row) => row.id !== item.id);
  const index = visible.findIndex((row) => row.id === item.id);
  const next = visible[index + 1];
  if (next) return siblings.findIndex((row) => row.id === next.id);
  const previous = visible[index - 1];
  return previous ? siblings.findIndex((row) => row.id === previous.id) + 1 : siblings.length;
}
