import { describe, expect, it } from "vitest";
import {
  clubFeedPreview,
  countEntries,
  defaultSpoilerUpto,
  excerpt,
  guardLine,
  positionMarker,
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
    expect(positionMarker(entry({ status_at: "did_not_finish" }))).toBe("Stopped");
    expect(positionMarker(entry({ status_at: "want_to_read" }))).toBe("before starting");
    expect(positionMarker(entry())).toBe("");
    expect(positionMarker(entry({ status_at: "currently_reading" }))).toBe("");
  });
});

describe("guardLine", () => {
  it("names the flag and where the reader is", () => {
    expect(guardLine({ kind: "marked", upto: 70, me: 35 }, entry())).toBe("Spoiler up to 70% · you're at 35%");
    expect(guardLine({ kind: "marked", upto: 100, me: 0 }, entry())).toBe("Spoiler: whole book · you're at 0%");
  });

  it("says where the writer was, with their name when asked", () => {
    const ahead = entry({ status_at: "currently_reading", progress_at: 35 });
    expect(guardLine({ kind: "ahead", at: 35, me: 0 }, ahead, "jonas")).toBe("jonas · at 35% · you're at 0%");
    expect(guardLine({ kind: "ahead", at: 35, me: 0 }, ahead)).toBe("at 35% · you're at 0%");
    const done = entry({ status_at: "finished", progress_at: 100, author_rating: 4 });
    expect(guardLine({ kind: "ahead", at: 100, me: 20 }, done, "tim")).toBe("tim · Finished ★★★★☆ · you're at 20%");
    expect(guardLine({ kind: "open" }, ahead)).toBe("");
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
    expect(spoilerLabel(100)).toBe("Spoiler: whole book");
    expect(spoilerLabel(35)).toBe("Spoiler up to 35%");
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
      "Spoiler up to 80% · you're at 20%",
    );
    expect(clubFeedPreview(feedItem())).toBe("Spoiler up to 80% · you're at 0%");
    expect(clubFeedPreview(feedItem({ my_progress: null, my_status: "want_to_read" }))).toBe(
      "Spoiler up to 80% · you're at 0%",
    );
  });

  it("hides the excerpt of an unflagged post written further on", () => {
    const ahead = { spoiler_upto: null, status_at: "currently_reading" as const, progress_at: 35 };
    expect(clubFeedPreview(feedItem({ entry: ahead, my_progress: 10, my_status: "currently_reading" }))).toBe(
      "at 35% · you're at 10%",
    );
    expect(clubFeedPreview(feedItem({ entry: ahead, my_progress: 35, my_status: "currently_reading" }))).toBe(secret);
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
