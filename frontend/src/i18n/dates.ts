import { i18n, type Locale } from "./index";

const BCP47: Record<Locale, string> = {
  en: "en-GB",
  de: "de-DE",
};

export function formatClubDate(
  iso: string | null | undefined,
  timeZone: string,
  locale: Locale,
): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  try {
    return new Intl.DateTimeFormat(BCP47[locale], {
      weekday: "short",
      day: "numeric",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      timeZone: timeZone || "UTC",
      timeZoneName: "short",
    }).format(date);
  } catch {
    return date.toISOString();
  }
}

/** "12 Mar 2026": a day, without the time, in `timeZone`. */
export function formatDay(
  iso: string | null | undefined,
  timeZone: string,
  locale: Locale,
): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  try {
    return new Intl.DateTimeFormat(BCP47[locale], {
      day: "numeric",
      month: "short",
      year: "numeric",
      timeZone: timeZone || "UTC",
    }).format(date);
  } catch {
    return date.toISOString().slice(0, 10);
  }
}

/**
 * The calendar day of `date` in `timeZone` (the device's zone without one, or
 * for a zone Intl does not know), as a UTC midnight.
 */
export function calendarDay(date: Date, timeZone?: string): number {
  if (timeZone) {
    try {
      const parts = new Intl.DateTimeFormat("en-CA", {
        timeZone, year: "numeric", month: "2-digit", day: "2-digit",
      }).formatToParts(date);
      const part = (type: string) => Number(parts.find((p) => p.type === type)?.value);
      return Date.UTC(part("year"), part("month") - 1, part("day"));
    } catch {
      // An unknown zone: count in the device's.
    }
  }
  return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
}

/** `timeZone` when Intl knows it, else undefined (the device's zone). */
function knownZone(timeZone: string): string | undefined {
  if (!timeZone) return "UTC";
  try {
    new Intl.DateTimeFormat("en", { timeZone });
    return timeZone;
  } catch {
    return undefined;
  }
}

const MINUTE = 60_000;
const DAY = 86_400_000;

/**
 * When a post was written, relative to `now`: "gerade eben", "vor 5 Min.",
 * "vor 3 Std." (the same day), "gestern", a weekday for 2 to 6 days, then
 * "12. Okt." this year and "12. Okt. 2025" before. Days are counted in
 * `timeZone`, the zone the row's exact time shows in, so "gestern" matches the
 * date in the title. A time in the future (clock skew) is "gerade eben". An
 * invalid `iso` gives "", so the caller can fall back to the server's label.
 */
export function formatRelative(
  iso: string | null | undefined,
  timeZone: string,
  locale: Locale,
  now: number = Date.now(),
): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const ms = now - date.getTime();
  if (ms < MINUTE) return String(i18n.global.t("relative.justNow", {}, { locale }));
  const zone = knownZone(timeZone);
  const tag = BCP47[locale];
  try {
    const minutes = Math.floor(ms / MINUTE);
    const rtf = new Intl.RelativeTimeFormat(tag, { numeric: "auto", style: "short" });
    if (minutes < 60) return rtf.format(-minutes, "minute");
    const today = new Date(now);
    const days = Math.round((calendarDay(today, zone) - calendarDay(date, zone)) / DAY);
    if (days <= 0) return rtf.format(-Math.floor(minutes / 60), "hour");
    if (days === 1) return rtf.format(-1, "day");
    // From 2 days on a weekday, never "vorgestern". The short German weekday
    // alone is "Mo" without a period, so the long one reads better.
    if (days < 7) return new Intl.DateTimeFormat(tag, { weekday: "long", timeZone: zone }).format(date);
    const year = (value: Date) =>
      new Intl.DateTimeFormat("en", { year: "numeric", timeZone: zone }).format(value);
    const sameYear = year(date) === year(today);
    return new Intl.DateTimeFormat(tag, {
      day: "numeric",
      month: "short",
      ...(sameYear ? {} : { year: "numeric" }),
      timeZone: zone,
    }).format(date);
  } catch {
    return "";
  }
}
