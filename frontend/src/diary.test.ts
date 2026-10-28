import { describe, expect, it, vi } from "vitest";
import {
  clubFeedPreview,
  countEntries,
  defaultSpoilerUpto,
  excerpt,
  isOwnDiaryEntry,
  isShielded,
  positionMarker,
  scanFeed,
  spoilerLabel,
} from "./diary";
import { post } from "./test/fixtures";
import type { DiaryEntry, DiaryFeedItem } from "./types";

function entry(overrides: Partial<DiaryEntry> = {}): DiaryEntry {
  return post({
    body: "hello",
    created_at: "2026-09-01T12:00:00Z",
    created_label: "Sep 1",
    ...overrides,
  });
}

describe("positionMarker", () => {
  it("describes where the author was", () => {
    expect(positionMarker(entry({ status_at: "currently_reading", progress_at: 45 }))).toBe("at 45%");
    expect(positionMarker(entry({ status_at: "finished", author_rating: 4 }))).toBe("Finished ★★★★☆");
    expect(positionMarker(entry({ status_at: "finished" }))).toBe("Finished");
    expect(positionMarker(entry({ status_at: "did_not_finish" }))).toBe("DNF");
    expect(positionMarker(entry({ status_at: "want_to_read" }))).toBe("before starting");
    expect(positionMarker(entry())).toBe("");
    expect(positionMarker(entry({ status_at: "currently_reading" }))).toBe("");
  });
});

describe("isShielded", () => {
  const flagged = entry({ spoiler_upto: 60 });

  it("hides flagged entries from readers who are behind", () => {
    expect(isShielded(flagged, 25, "currently_reading")).toBe(true);
    expect(isShielded(flagged, null, null)).toBe(true);
    expect(isShielded(flagged, 60, "currently_reading")).toBe(false);
    expect(isShielded(flagged, 80, "currently_reading")).toBe(false);
  });

  it("never shields the writer, finished or DNF readers, unflagged or deleted entries", () => {
    expect(isShielded(entry({ spoiler_upto: 100, mine: true }), 40, "currently_reading")).toBe(false);
    expect(isShielded(flagged, null, "finished")).toBe(false);
    expect(isShielded(flagged, 10, "did_not_finish")).toBe(false);
    expect(isShielded(entry(), null, null)).toBe(false);
    expect(isShielded(entry({ spoiler_upto: 90, deleted: true }), 0, null)).toBe(false);
  });
});

describe("composer defaults", () => {
  it("proposes the writer's own position", () => {
    expect(defaultSpoilerUpto(40, "currently_reading")).toBe(40);
    expect(defaultSpoilerUpto(null, "finished")).toBe(100);
    expect(defaultSpoilerUpto(null, "want_to_read")).toBeNull();
    expect(defaultSpoilerUpto(null, null)).toBeNull();
  });

  it("labels the flag", () => {
    expect(spoilerLabel(null)).toBe("No spoiler flag");
    expect(spoilerLabel(100)).toBe("Whole book");
    expect(spoilerLabel(35)).toBe("Safe up to 35%");
  });
});

describe("clubFeedPreview", () => {
  const secret = "The coconut pillow on p. 237 is a tell.";

  function feedItem(
    overrides: {
      entry?: Partial<DiaryEntry>;
      my_progress?: number | null;
      my_status?: DiaryFeedItem["my_status"];
    } = {},
  ): Pick<DiaryFeedItem, "entry" | "my_progress" | "my_status"> {
    return {
      entry: entry({ body: secret, spoiler_upto: 80, ...overrides.entry }),
      my_progress: overrides.my_progress ?? null,
      my_status: overrides.my_status ?? null,
    };
  }

  it("hides the excerpt when the viewer is behind or has not shelved the book", () => {
    expect(clubFeedPreview(feedItem({ my_progress: 20, my_status: "currently_reading" }))).toBe(
      "Spoiler-flagged note",
    );
    expect(clubFeedPreview(feedItem())).toBe("Spoiler-flagged note");
    expect(clubFeedPreview(feedItem({ my_progress: null, my_status: "want_to_read" }))).toBe(
      "Spoiler-flagged note",
    );
  });

  it("shows the excerpt to the writer, finished or DNF readers, and readers past the flag", () => {
    expect(clubFeedPreview(feedItem({ entry: { mine: true }, my_progress: 10 }))).toBe(secret);
    expect(clubFeedPreview(feedItem({ my_status: "finished" }))).toBe(secret);
    expect(clubFeedPreview(feedItem({ my_progress: 5, my_status: "did_not_finish" }))).toBe(secret);
    expect(clubFeedPreview(feedItem({ my_progress: 80, my_status: "currently_reading" }))).toBe(
      secret,
    );
    expect(clubFeedPreview(feedItem({ entry: { spoiler_upto: null } }))).toBe(secret);
  });
});

describe("excerpt and counting", () => {
  it("flattens whitespace and trims with an ellipsis", () => {
    expect(excerpt("a  b\n\n c")).toBe("a b c");
    const long = excerpt("x".repeat(200), 20);
    expect(long).toHaveLength(20);
    expect(long.endsWith("…")).toBe(true);
  });

  it("counts entries and replies, skipping tombstones", () => {
    const items = [
      entry({ replies: [entry({ id: 2, parent_id: 1 })] }),
      entry({ id: 3, deleted: true, replies: [entry({ id: 4, parent_id: 3 })] }),
    ];
    expect(countEntries(items)).toBe(3);
  });
});

describe("the reader's own diary (/diary)", () => {
  it("keeps the reader's own diary entries and replies, not club posts or tombstones", () => {
    const club = { slug: "readers-ab12", name: "Readers" };
    expect(isOwnDiaryEntry({ entry: entry({ mine: true }) })).toBe(true);
    expect(isOwnDiaryEntry({ entry: entry({ mine: true, parent_id: 4 }) })).toBe(true);
    expect(isOwnDiaryEntry({ entry: entry({ mine: false }) })).toBe(false);
    expect(isOwnDiaryEntry({ entry: entry({ mine: true, club }) })).toBe(false);
    expect(isOwnDiaryEntry({ entry: entry({ mine: true, deleted: true }) })).toBe(false);
  });

  const row = (id: number, mine: boolean) => ({ entry: { id, mine } });
  const keepMine = (item: { entry: { mine: boolean } }) => item.entry.mine;

  it("reads on until it found enough of the reader's entries", async () => {
    const pages = [
      { items: [row(30, false), row(29, true), row(28, false)], has_more: true },
      { items: [row(27, false), row(26, true), row(25, true)], has_more: true },
      { items: [row(24, true)], has_more: false },
    ];
    const load = vi.fn((_before: number | undefined) => Promise.resolve(pages.shift()!));
    const page = await scanFeed(load, keepMine, { want: 3, maxPages: 5 });
    expect(page.items.map((item) => item.entry.id)).toEqual([29, 26, 25]);
    expect(load.mock.calls.map(([before]) => before)).toEqual([undefined, 28]);
    // The next page starts after the last id read, not the last one kept.
    expect(page.cursor).toBe(25);
    expect(page.hasMore).toBe(true);
  });

  it("stops at the end of the feed and after the page budget", async () => {
    const end = await scanFeed(
      () => Promise.resolve({ items: [row(5, false)], has_more: false }),
      keepMine,
      { before: 9, want: 10, maxPages: 5 },
    );
    expect(end).toEqual({ items: [], hasMore: false, cursor: 5 });

    let next = 100;
    const load = vi.fn(() => Promise.resolve({ items: [row((next -= 1), false)], has_more: true }));
    const budget = await scanFeed(load, keepMine, { want: 10, maxPages: 3 });
    expect(load).toHaveBeenCalledTimes(3);
    expect(budget).toEqual({ items: [], hasMore: true, cursor: 97 });
  });
});
