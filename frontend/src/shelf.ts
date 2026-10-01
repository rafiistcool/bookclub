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
