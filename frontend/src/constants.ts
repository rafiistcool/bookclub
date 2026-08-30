export const STATUSES = [
  "want_to_read",
  "currently_reading",
  "finished",
  "did_not_finish",
] as const;

export type Status = (typeof STATUSES)[number];

export const STATUS_LABEL: Record<Status, string> = {
  want_to_read: "Want to read",
  currently_reading: "Reading",
  finished: "Finished",
  did_not_finish: "Did not finish",
};

export const STATUS_SHORT: Record<Status, string> = {
  want_to_read: "Want",
  currently_reading: "Reading",
  finished: "Done",
  did_not_finish: "DNF",
};

export type CoverSize = "xs" | "sm" | "md" | "lg" | "fluid";

/** Open Library's image tiers, mapped to the sizes we actually render. */
const COVER_TIER: Record<CoverSize, "S" | "M" | "L"> = {
  xs: "S",
  sm: "S",
  md: "M",
  lg: "L",
  fluid: "L",
};

export function coverUrl(
  coverId: number | null | undefined,
  size: CoverSize = "fluid",
): string | null {
  if (!coverId) return null;
  return `https://covers.openlibrary.org/b/id/${coverId}-${COVER_TIER[size]}.jpg`;
}

/** "/works/OL1W" -> "OL1W", the form the detail route and API path take. */
export function workId(olWorkKey: string): string {
  return olWorkKey.replace(/^\/works\//, "");
}

export function bookPath(olWorkKey: string): string {
  return `/book/${workId(olWorkKey)}`;
}

export function openLibraryUrl(olWorkKey: string): string {
  return `https://openlibrary.org${olWorkKey}`;
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

/** "in 3 days" / "today" / "2 weeks ago", for meeting dates. */
export function relativeDay(iso: string | null | undefined): string {
  if (!iso) return "";
  const target = new Date(iso);
  if (Number.isNaN(target.getTime())) return "";
  const startOfDay = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round(
    (startOfDay(target) - startOfDay(new Date())) / 86_400_000,
  );
  if (days === 0) return "today";
  if (days === 1) return "tomorrow";
  if (days === -1) return "yesterday";
  if (days > 0) return days < 14 ? `in ${days} days` : `in ${Math.round(days / 7)} weeks`;
  const ago = Math.abs(days);
  return ago < 14 ? `${ago} days ago` : `${Math.round(ago / 7)} weeks ago`;
}
