import type { Status } from "./constants";
import { starLabel } from "./constants";
import { t } from "./i18n";
import { spoilerGuard, type SpoilerGuard } from "./spoilers";
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

export const EXCERPT_LENGTH = 140;

/** Trim to a single line of at most `max` characters, ellipsis included. */
export function excerpt(body: string, max = EXCERPT_LENGTH): string {
  const flat = body.replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max - 1).trimEnd()}…`;
}

/**
 * The one quiet line that stands in for a post the reader is not far enough
 * for (`spoilers.ts`): "Spoiler bis 70 % · du bist bei 35 %" for a flagged
 * post, "bei 35 % · du bist bei 0 %" for one written further on. With
 * `author`, the line starts with the writer's name.
 */
export function guardLine(
  guard: SpoilerGuard,
  entry: { progress_at: number | null; status_at: Status | null; author_rating: number | null },
  author?: string,
): string {
  if (guard.kind === "open") return "";
  if (guard.kind === "marked") {
    return guard.upto >= 100
      ? t("spoilers.markedWhole", { me: guard.me })
      : t("spoilers.marked", { upto: guard.upto, me: guard.me });
  }
  const where = positionMarker(entry) || t("diary.atPercent", { n: guard.at });
  return author != null
    ? t("spoilers.aheadBy", { name: author, where, me: guard.me })
    : t("spoilers.ahead", { where, me: guard.me });
}

/**
 * Club → Recently written preview. A post the reader is not far enough for
 * never leaks its body: the guard's line stands in for it.
 */
export function clubFeedPreview(
  item: {
    entry: {
      body: string;
      spoiler_upto: number | null;
      progress_at: number | null;
      status_at: Status | null;
      author_rating: number | null;
      deleted: boolean;
      mine: boolean;
    };
    my_progress: number | null;
    my_status: Status | null;
  },
): string {
  const guard = spoilerGuard(item.entry, item.my_progress, item.my_status);
  return guard.kind === "open" ? excerpt(item.entry.body) : guardLine(guard, item.entry);
}

export function countEntries(items: DiaryEntry[]): number {
  return items.reduce(
    (sum, entry) => sum + (entry.deleted ? 0 : 1) + entry.replies.length,
    0,
  );
}

