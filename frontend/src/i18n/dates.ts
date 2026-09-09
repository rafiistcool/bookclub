import type { Locale } from "./index";

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
