import { t } from "./i18n";

/** "YYYY-MM-DD" for a date in the device's zone, to compare with release dates. */
export function localDay(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/**
 * The line under a new release: "Out 12 Nov 2026" for one still to come,
 * "Since 3 Oct 2026" for one already out. Empty for a missing or bad date.
 */
export function releaseNote(date: string | null | undefined, locale: string, today: string = localDay()): string {
  if (!date || !/^\d{4}-\d{2}-\d{2}$/.test(date)) return "";
  const parsed = new Date(`${date}T12:00:00Z`);
  if (Number.isNaN(parsed.getTime())) return "";
  const label = new Intl.DateTimeFormat(locale === "de" ? "de-DE" : "en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }).format(parsed);
  return t(date > today ? "discover.releaseUpcoming" : "discover.releaseOut", { date: label });
}
