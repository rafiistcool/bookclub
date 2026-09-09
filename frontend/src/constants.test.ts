import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bookPath,
  catalogUrl,
  coverUrl,
  editionCoverUrl,
  extractIsbn,
  googleBooksUrl,
  isbnCoverUrl,
  isbnFromWorkKey,
  isClubWorkId,
  isGoogleWorkId,
  isIsbnWorkId,
  isOpenLibraryWorkId,
  monogram,
  openLibraryUrl,
  relativeDay,
  starLabel,
  workId,
} from "./constants";

describe("covers and links", () => {
  it("maps our sizes onto Open Library's image tiers", () => {
    expect(coverUrl(123, "xs")).toBe(
      "https://covers.openlibrary.org/b/id/123-S.jpg?default=false",
    );
    expect(coverUrl(123, "md")).toBe(
      "https://covers.openlibrary.org/b/id/123-M.jpg?default=false",
    );
    expect(coverUrl(123, "tile")).toBe(
      "https://covers.openlibrary.org/b/id/123-M.jpg?default=false",
    );
    expect(coverUrl(123)).toBe("https://covers.openlibrary.org/b/id/123-L.jpg?default=false");
    expect(coverUrl(null)).toBeNull();
    expect(coverUrl(0)).toBeNull();
    expect(coverUrl(-1)).toBeNull();
  });

  it("builds edition and ISBN fallbacks", () => {
    expect(editionCoverUrl("OL1M", "sm")).toBe(
      "https://covers.openlibrary.org/b/olid/OL1M-S.jpg?default=false",
    );
    expect(isbnCoverUrl("978-0-316-76948-8", "md")).toBe(
      "https://covers.openlibrary.org/b/isbn/9780316769488-M.jpg?default=false",
    );
    expect(extractIsbn("9780316769488")).toBe("9780316769488");
    expect(extractIsbn("it")).toBeNull();
  });

  it("derives the detail route and Open Library link from a work key", () => {
    expect(workId("/works/OL1W")).toBe("OL1W");
    expect(workId("OL1W")).toBe("OL1W");
    expect(bookPath("/works/OL1W")).toBe("/book/OL1W");
    expect(openLibraryUrl("/works/OL1W")).toBe("https://openlibrary.org/works/OL1W");
    expect(isClubWorkId("BCdeadbeef01")).toBe(true);
    expect(openLibraryUrl("/works/BCdeadbeef01")).toBeNull();
    expect(isOpenLibraryWorkId("OL1W")).toBe(true);
    expect(isIsbnWorkId("ISBN9780316769488")).toBe(true);
    expect(isGoogleWorkId("GBzyTCAlFPjgYC")).toBe(true);
    expect(isbnFromWorkKey("/works/ISBN9780316769488")).toBe("9780316769488");
    expect(openLibraryUrl("/works/ISBN9780316769488")).toBeNull();
    expect(googleBooksUrl("/works/GBzyTCAlFPjgYC")).toBe(
      "https://books.google.com/books?id=zyTCAlFPjgYC",
    );
    expect(catalogUrl("/works/ISBN9780316769488")).toContain("ISBN9780316769488");
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
