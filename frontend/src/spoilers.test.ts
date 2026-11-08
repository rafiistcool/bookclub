import { describe, expect, it } from "vitest";
import { isGuarded, readerPercent, spoilerGuard } from "./spoilers";

function post(overrides: Partial<Parameters<typeof spoilerGuard>[0]> = {}) {
  return { spoiler_upto: null, progress_at: null, deleted: false, mine: false, ...overrides };
}

describe("readerPercent", () => {
  it("is 100 finished, the progress while reading, else 0", () => {
    expect(readerPercent(null, "finished")).toBe(100);
    expect(readerPercent(35, "currently_reading")).toBe(35);
    expect(readerPercent(null, "currently_reading")).toBe(0);
    expect(readerPercent(null, "want_to_read")).toBe(0);
    expect(readerPercent(null, null)).toBe(0);
  });
});

describe("spoilerGuard", () => {
  it("folds a post written further on than the reader is", () => {
    expect(spoilerGuard(post({ progress_at: 35 }), 0, "currently_reading")).toEqual({ kind: "ahead", at: 35, me: 0 });
    // Not on the shelf (or only wanted): 0 %.
    expect(spoilerGuard(post({ progress_at: 35 }), null, null)).toEqual({ kind: "ahead", at: 35, me: 0 });
    expect(spoilerGuard(post({ progress_at: 35 }), 20, "want_to_read")).toEqual({ kind: "ahead", at: 35, me: 0 });
    // A writer who had finished wrote at 100 %.
    expect(spoilerGuard(post({ progress_at: 100 }), 80, "currently_reading")).toEqual({ kind: "ahead", at: 100, me: 80 });
  });

  it("opens a post written at or before the reader's place", () => {
    expect(spoilerGuard(post({ progress_at: 35 }), 35, "currently_reading")).toEqual({ kind: "open" });
    expect(spoilerGuard(post({ progress_at: 10 }), 35, "currently_reading")).toEqual({ kind: "open" });
    // Written before starting, or without a known place.
    expect(spoilerGuard(post(), 0, null)).toEqual({ kind: "open" });
  });

  it("never folds one's own posts, deleted ones, or anything for a reader who finished", () => {
    expect(isGuarded(post({ progress_at: 90, mine: true }), 0, null)).toBe(false);
    expect(isGuarded(post({ progress_at: 90, spoiler_upto: 90, deleted: true }), 0, null)).toBe(false);
    expect(isGuarded(post({ progress_at: 100, spoiler_upto: 100 }), null, "finished")).toBe(false);
    // Given up: the reader is done with the book, as before.
    expect(isGuarded(post({ progress_at: 100, spoiler_upto: 60 }), 10, "did_not_finish")).toBe(false);
  });

  it("follows the writer's flag both ways", () => {
    const flagged = post({ spoiler_upto: 60, progress_at: 80 });
    expect(spoilerGuard(flagged, 25, "currently_reading")).toEqual({ kind: "marked", upto: 60, me: 25 });
    expect(spoilerGuard(flagged, null, null)).toEqual({ kind: "marked", upto: 60, me: 0 });
    // Past the flag the post is open, although its writer was further on.
    expect(spoilerGuard(flagged, 60, "currently_reading")).toEqual({ kind: "open" });
    expect(spoilerGuard(flagged, 70, "currently_reading")).toEqual({ kind: "open" });
  });
});
