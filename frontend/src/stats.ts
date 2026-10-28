import type { MonthCount } from "./api/generated";
import type { Locale } from "./i18n";

const BCP47: Record<Locale, string> = { en: "en-GB", de: "de-DE" };

/** All twelve months of `year` (`YYYY-MM`), with the server's counts and 0 for the rest. */
export function monthSeries(year: number, rows: readonly MonthCount[]): MonthCount[] {
  const counts = new Map(rows.map((row) => [row.month, row.finished]));
  return Array.from({ length: 12 }, (_, index) => {
    const month = `${year}-${String(index + 1).padStart(2, "0")}`;
    return { month, finished: counts.get(month) ?? 0 };
  });
}

/** "Jan" / "Jan." for a `YYYY-MM` month (the server counts months in UTC). */
export function monthName(month: string, locale: Locale, style: "short" | "long" = "short"): string {
  const [year, number] = month.split("-").map(Number);
  if (!year || !number) return month;
  return new Intl.DateTimeFormat(BCP47[locale], { month: style, timeZone: "UTC" }).format(
    new Date(Date.UTC(year, number - 1, 1)),
  );
}

/** A count with the locale's grouping: "12,480" / "12.480". */
export function formatCount(value: number, locale: Locale): string {
  return new Intl.NumberFormat(BCP47[locale]).format(value);
}

/** A mean rating with one decimal ("4.3" / "4,3"); "–" without ratings. */
export function formatRating(value: number | null | undefined, locale: Locale): string {
  if (value == null) return "–";
  return new Intl.NumberFormat(BCP47[locale], { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(value);
}
