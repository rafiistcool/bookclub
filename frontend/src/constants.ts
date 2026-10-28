import { t } from "./i18n";

export const FAVORITE_LIMIT = 3;

export const STATUSES = [
  "want_to_read",
  "currently_reading",
  "finished",
  "did_not_finish",
] as const;

export type Status = (typeof STATUSES)[number];

export function statusLabel(status: Status): string {
  return t(`status.${status}`);
}

export function statusShort(status: Status): string {
  return t(`statusShort.${status}`);
}

export type CoverSize = "xs" | "sm" | "md" | "lg" | "fluid" | "tile";

const ISBN_RE = /^(?:\d{9}[\dXx]|\d{13})$/;
const OL_WORK_ID_RE = /^OL\d+W$/;
const NUMERIC_ID_RE = /^[1-9]\d{0,15}$/;
/** Book keys of the legacy app's `/book/<key>` links: Open Library, club-added, Google ISBN and volume. */
const LEGACY_WORK_KEY_RE = /^(?:OL\d+W|BC[a-f0-9]{10}|ISBN[\dXx]{10,13}|isbn[\dXx]{10,13}|GB[A-Za-z0-9_-]{1,40})$/;

export function extractIsbn(raw: string): string | null {
  const cleaned = raw.replace(/[\s-]/g, "").toUpperCase();
  return ISBN_RE.test(cleaned) ? cleaned : null;
}

/** The book page of a work. Every link to a book goes through here. */
export function bookPath(book: number | { id: number }): string {
  return `/book/${typeof book === "number" ? book : book.id}`;
}

export type BookParam =
  | { kind: "id"; id: number }
  | { kind: "legacy"; key: string }
  | { kind: "invalid" };

/**
 * What `/book/:id` names: a numeric work id, or a key of the legacy app
 * (`OL…W`, `BC…`, `ISBN…`, `GB…`) that the server resolves to one.
 */
export function parseBookParam(raw: string): BookParam {
  if (NUMERIC_ID_RE.test(raw)) {
    const id = Number(raw);
    if (Number.isSafeInteger(id)) return { kind: "id", id };
  }
  if (LEGACY_WORK_KEY_RE.test(raw)) return { kind: "legacy", key: raw };
  return { kind: "invalid" };
}

/** "/works/OL1W" -> the work's Open Library page. Other works have none. */
export function openLibraryUrl(olWorkKey: string | null | undefined): string | null {
  const id = (olWorkKey || "").replace(/^\/works\//, "");
  if (!OL_WORK_ID_RE.test(id)) return null;
  return `https://openlibrary.org/works/${id}`;
}

export type FinishNote = {
  rating?: number | null;
  take?: string;
  dnf_reason?: string;
  progress?: number | null;
};

export function starLabel(rating: number | null | undefined): string {
  if (!rating) return "";
  return "★".repeat(rating) + "☆".repeat(5 - rating);
}

export function monogram(title: string): string {
  const letter = title.trim().charAt(0);
  return letter ? letter.toUpperCase() : "?";
}

/** The calendar day of `date` in `timeZone` (the device's zone without one), as a UTC midnight. */
function calendarDay(date: Date, timeZone?: string): number {
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

/**
 * "in 3 days" / "today" / "2 weeks ago", for meeting dates. Days are counted
 * in `timeZone`, the club's for a meeting, so "tomorrow" matches the date the
 * club sees.
 */
export function relativeDay(iso: string | null | undefined, timeZone?: string): string {
  if (!iso) return "";
  const target = new Date(iso);
  if (Number.isNaN(target.getTime())) return "";
  const days = Math.round(
    (calendarDay(target, timeZone) - calendarDay(new Date(), timeZone)) / 86_400_000,
  );
  if (days === 0) return t("relative.today");
  if (days === 1) return t("relative.tomorrow");
  if (days === -1) return t("relative.yesterday");
  if (days > 0) {
    return days < 14
      ? t("relative.inDays", { n: days })
      : t("relative.inWeeks", { n: Math.round(days / 7) });
  }
  const ago = Math.abs(days);
  return ago < 14
    ? t("relative.daysAgo", { n: ago })
    : t("relative.weeksAgo", { n: Math.round(ago / 7) });
}

/** A book typed or pasted as a link (`…/book/42`, `/book/42#entry-3`) or a bare id; null for anything else. */
export function bookIdFromInput(text: string): number | null {
  const value = text.trim();
  const match = /^(?:\d+|(?:https?:\/\/[^/\s]+)?\/book\/(\d+)(?:[?#].*)?)$/.exec(value);
  if (!match) return null;
  const id = Number(match[1] ?? value);
  return Number.isSafeInteger(id) && id > 0 ? id : null;
}
