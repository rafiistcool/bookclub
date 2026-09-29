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

export type ShelfGroup = { key: string; status: Status; month: string; items: ShelfItem[] };

export function shelfGroups(items: ShelfItem[], timeZone: string, includeCurrentMonth = false): ShelfGroup[] {
  const sorted = [...items].sort((a, b) => a.position - b.position || a.id - b.id);
  return STATUSES.flatMap((status): ShelfGroup[] => {
    const rows = sorted.filter((item) => item.status === status);
    if (status !== "finished") return [{ key: status, status, month: "", items: rows }];
    const months = new Map<string, ShelfItem[]>();
    if (includeCurrentMonth) months.set(clubDate(new Date().toISOString(), timeZone).slice(0, 7), []);
    for (const item of rows) {
      const month = clubDate(item.finished_at, timeZone).slice(0, 7);
      if (!months.has(month)) months.set(month, []);
      months.get(month)!.push(item);
    }
    return [...months.keys()].sort().reverse().map((month) => ({
      key: `finished:${month}`, status, month, items: months.get(month)!,
    }));
  });
}

// Month groups are only a view over the existing per-status order. Translate
// the drop's visible neighbours to the position expected by the shelf API.
export function dropPosition(all: ShelfItem[], rows: ShelfItem[], item: ShelfItem, status: Status): number {
  const siblings = all.filter((row) => row.status === status && row.id !== item.id)
    .sort((a, b) => a.position - b.position || a.id - b.id);
  const index = rows.findIndex((row) => row.id === item.id);
  const next = rows[index + 1];
  if (next) return siblings.findIndex((row) => row.id === next.id);
  const previous = rows[index - 1];
  return previous ? siblings.findIndex((row) => row.id === previous.id) + 1 : siblings.length;
}
