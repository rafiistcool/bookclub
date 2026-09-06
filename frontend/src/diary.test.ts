import { describe, expect, it } from "vitest";
import {
  countEntries,
  defaultSpoilerUpto,
  excerpt,
  isShielded,
  positionMarker,
  spoilerLabel,
} from "./diary";
import type { DiaryEntry } from "./types";

function entry(overrides: Partial<DiaryEntry> = {}): DiaryEntry {
  return {
    id: 1,
    author: "ada",
    mine: false,
    body: "hello",
    deleted: false,
    spoiler_upto: null,
    progress_at: null,
    status_at: null,
    author_rating: null,
    parent_id: null,
    created_at: "2026-09-01T12:00:00Z",
    created_label: "Sep 1",
    edited: false,
    reactions: [],
    replies: [],
    ...overrides,
  };
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
