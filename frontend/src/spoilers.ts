import type { Status } from "./constants";

/**
 * Spoiler protection by the app (critique 2026-11-07, P2): the app knows how
 * far the writer was and how far the reader is, so it protects the reader
 * without waiting for the writer to flag anything.
 *
 * - `marked`: the writer flagged the post as a spoiler up to `upto` %, and
 *   the reader is below that. The writer's flag is the more precise word and
 *   wins over the automatic rule both ways: a post flagged up to 30 % is open
 *   to a reader at 35 %, even when its writer was at 80 %.
 * - `ahead`: no flag, but the post was written further on (`progress_at`;
 *   100 for a writer who had finished) than the reader is now.
 * - `open`: everything else. One's own posts, deleted ones, and every post for
 *   a reader who finished the book (or gave it up) are always open.
 *
 * The reader's position: finished is 100 %, reading is their progress, any
 * other status (or no shelf entry) is 0 %.
 */
export type SpoilerGuard =
  | { kind: "open" }
  | { kind: "marked"; upto: number; me: number }
  | { kind: "ahead"; at: number; me: number };

export type GuardedPost = {
  spoiler_upto: number | null;
  progress_at: number | null;
  deleted: boolean;
  mine: boolean;
};

/** How far the reader is, in percent: finished 100, reading their progress, else 0. */
export function readerPercent(progress: number | null, status: Status | null): number {
  if (status === "finished") return 100;
  if (status === "currently_reading") return Math.min(100, Math.max(0, progress ?? 0));
  return 0;
}

export function spoilerGuard(post: GuardedPost, myProgress: number | null, myStatus: Status | null): SpoilerGuard {
  if (post.deleted || post.mine) return { kind: "open" };
  if (myStatus === "finished" || myStatus === "did_not_finish") return { kind: "open" };
  const me = readerPercent(myProgress, myStatus);
  if (post.spoiler_upto != null) {
    return me < post.spoiler_upto ? { kind: "marked", upto: post.spoiler_upto, me } : { kind: "open" };
  }
  if (post.progress_at != null && post.progress_at > me) return { kind: "ahead", at: post.progress_at, me };
  return { kind: "open" };
}

/** Whether the post's text waits for a tap. */
export function isGuarded(post: GuardedPost, myProgress: number | null, myStatus: Status | null): boolean {
  return spoilerGuard(post, myProgress, myStatus).kind !== "open";
}
