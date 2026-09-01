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

/** The status a member most likely moves to from the current one. */
export const STATUS_NEXT: Record<Status, Status> = {
  want_to_read: "currently_reading",
  currently_reading: "finished",
  finished: "want_to_read",
  did_not_finish: "want_to_read",
};

export const SUBJECTS = [
  { label: "Fiction", value: "fiction" },
  { label: "Fantasy", value: "fantasy" },
  { label: "Mystery", value: "mystery" },
  { label: "Romance", value: "romance" },
  { label: "Sci-Fi", value: "science_fiction" },
  { label: "History", value: "history" },
  { label: "Biography", value: "biography" },
  { label: "Horror", value: "horror" },
  { label: "YA", value: "young_adult" },
] as const;

export const REACTIONS = ["❤️", "👍", "😂", "😮", "🔥", "📚"] as const;

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
  return `${rating} of 5`;
}

export function monogram(title: string): string {
  const letter = title.trim().charAt(0);
  return letter ? letter.toUpperCase() : "?";
}

const AVATAR_HUES = [12, 28, 44, 92, 160, 190, 215, 250, 285, 320, 345];

/** Deterministic hue per member so a name always gets the same colour everywhere. */
export function avatarColor(username: string): string {
  let hash = 0;
  for (const char of username) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  const hue = AVATAR_HUES[hash % AVATAR_HUES.length];
  return `hsl(${hue} 42% 44%)`;
}

export function initials(username: string): string {
  const clean = username.replace(/[^a-z0-9]/gi, "");
  return (clean.slice(0, 2) || "?").toUpperCase();
}

const DAY = 86_400_000;
const HOUR = 3_600_000;
const MINUTE = 60_000;

export type Countdown = {
  label: string;
  soon: boolean;
  past: boolean;
};

/** "Meeting in 9 days", "Tomorrow", "Tonight", "3 hours ago". */
export function countdown(iso: string | null | undefined, now: Date = new Date()): Countdown | null {
  if (!iso) return null;
  const target = new Date(iso);
  if (Number.isNaN(target.getTime())) return null;
  const diff = target.getTime() - now.getTime();
  const abs = Math.abs(diff);
  const past = diff < 0;
  let label: string;
  if (abs < MINUTE) label = past ? "just now" : "now";
  else if (abs < HOUR) {
    const minutes = Math.round(abs / MINUTE);
    label = past ? `${minutes} min ago` : `in ${minutes} min`;
  } else if (abs < DAY) {
    const hours = Math.round(abs / HOUR);
    label = past ? `${hours} h ago` : `in ${hours} h`;
  } else {
    const days = Math.round(abs / DAY);
    if (!past && days === 1) label = "tomorrow";
    else if (past && days === 1) label = "yesterday";
    else label = past ? `${days} days ago` : `in ${days} days`;
  }
  return { label, soon: !past && diff < 2 * DAY, past };
}

export function fromNow(iso: string, now: Date = new Date()): string {
  const result = countdown(iso, now);
  return result ? result.label : "";
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

export function excerpt(text: string, max = 120): string {
  const clean = text.replace(/\s+/g, " ").trim();
  return clean.length <= max ? clean : `${clean.slice(0, max - 1).trimEnd()}…`;
}
