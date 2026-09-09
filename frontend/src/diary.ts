import type { Status } from "./constants";
import { starLabel } from "./constants";
import { t } from "./i18n";
import type { DiaryEntry } from "./types";

export const REACTIONS = ["❤️", "👍", "😂", "😮", "🔥", "📚"] as const;

/** Where the author was when they wrote this: "at 45%", "Finished ★★★★☆", "DNF". */
export function positionMarker(entry: {
  progress_at: number | null;
  status_at: Status | null;
  author_rating: number | null;
}): string {
  if (entry.status_at === "finished") {
    const stars = starLabel(entry.author_rating);
    return stars ? t("diary.finishedStars", { stars }) : t("diary.finished");
  }
  if (entry.status_at === "did_not_finish") return t("diary.dnf");
  if (entry.status_at === "currently_reading" && entry.progress_at != null) {
    return t("diary.atPercent", { n: entry.progress_at });
  }
  if (entry.status_at === "want_to_read") return t("diary.beforeStarting");
  return "";
}

/**
 * True when the reader has not got far enough for this entry: the writer
 * flagged it safe only up to `spoiler_upto`, and the reader is below that.
 * Writers always see their own entries. Readers who finished (or DNF'd) see
 * everything; readers who never shelved the book count as being at 0%.
 */
export function isShielded(
  entry: { spoiler_upto: number | null; deleted: boolean; mine: boolean },
  myProgress: number | null,
  myStatus: Status | null,
): boolean {
  if (entry.deleted || entry.mine || entry.spoiler_upto == null) return false;
  if (myStatus === "finished" || myStatus === "did_not_finish") return false;
  return (myProgress ?? 0) < entry.spoiler_upto;
}

/** The composer's default "safe up to": where the writer currently is. */
export function defaultSpoilerUpto(myProgress: number | null, myStatus: Status | null): number | null {
  if (myStatus === "finished") return 100;
  if (myStatus === "currently_reading" && myProgress != null) return myProgress;
  return null;
}

export function spoilerLabel(upto: number | null): string {
  if (upto == null) return t("diary.noSpoiler");
  if (upto >= 100) return t("diary.wholeBook");
  return t("diary.safeUpto", { n: upto });
}

/** Trim to a single line of at most `max` characters, ellipsis included. */
export function excerpt(body: string, max = 140): string {
  const flat = body.replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max - 1).trimEnd()}…`;
}

/**
 * Club → Recently written preview. Shielded items never leak the body;
 * the badge already says they are spoiler-flagged.
 */
export function clubFeedPreview(
  item: {
    entry: { body: string; spoiler_upto: number | null; deleted: boolean; mine: boolean };
    my_progress: number | null;
    my_status: Status | null;
  },
): string {
  if (isShielded(item.entry, item.my_progress, item.my_status)) {
    return t("diary.spoilerPreview");
  }
  return excerpt(item.entry.body);
}

export function countEntries(items: DiaryEntry[]): number {
  return items.reduce(
    (sum, entry) => sum + (entry.deleted ? 0 : 1) + entry.replies.length,
    0,
  );
}
