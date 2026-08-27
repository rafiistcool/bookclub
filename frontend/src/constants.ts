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

export function coverUrl(
  coverId: number | null | undefined,
  size: "S" | "M" | "L" = "L",
): string | null {
  if (!coverId) return null;
  return `https://covers.openlibrary.org/b/id/${coverId}-${size}.jpg`;
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
