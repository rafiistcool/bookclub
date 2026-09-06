import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bookPath,
  coverUrl,
  monogram,
  openLibraryUrl,
  relativeDay,
  starLabel,
  workId,
} from "./constants";

describe("covers and links", () => {
  it("maps our sizes onto Open Library's image tiers", () => {
    expect(coverUrl(123, "xs")).toBe("https://covers.openlibrary.org/b/id/123-S.jpg");
    expect(coverUrl(123, "md")).toBe("https://covers.openlibrary.org/b/id/123-M.jpg");
    expect(coverUrl(123)).toBe("https://covers.openlibrary.org/b/id/123-L.jpg");
    expect(coverUrl(null)).toBeNull();
    expect(coverUrl(0)).toBeNull();
  });

  it("derives the detail route and Open Library link from a work key", () => {
    expect(workId("/works/OL1W")).toBe("OL1W");
    expect(workId("OL1W")).toBe("OL1W");
    expect(bookPath("/works/OL1W")).toBe("/book/OL1W");
    expect(openLibraryUrl("/works/OL1W")).toBe("https://openlibrary.org/works/OL1W");
  });
});

describe("labels", () => {
  it("builds monograms and star labels", () => {
    expect(monogram("  dune")).toBe("D");
    expect(monogram("")).toBe("?");
    expect(starLabel(3)).toBe("★★★☆☆");
    expect(starLabel(null)).toBe("");
  });
});

describe("relativeDay", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns an empty string for missing or invalid dates", () => {
    expect(relativeDay(null)).toBe("");
    expect(relativeDay("not a date")).toBe("");
  });

  it("describes distances in days, then weeks", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 1, 12, 0, 0));
    expect(relativeDay(new Date(2026, 8, 1, 20).toISOString())).toBe("today");
    expect(relativeDay(new Date(2026, 8, 2).toISOString())).toBe("tomorrow");
    expect(relativeDay(new Date(2026, 7, 31).toISOString())).toBe("yesterday");
    expect(relativeDay(new Date(2026, 8, 10).toISOString())).toBe("in 9 days");
    expect(relativeDay(new Date(2026, 8, 22).toISOString())).toBe("in 3 weeks");
    expect(relativeDay(new Date(2026, 7, 20).toISOString())).toBe("12 days ago");
    expect(relativeDay(new Date(2026, 7, 4).toISOString())).toBe("4 weeks ago");
  });
});
