import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bookIdFromInput,
  bookPath,
  extractIsbn,
  monogram,
  openLibraryUrl,
  parseBookParam,
  relativeDay,
  starLabel,
} from "./constants";

describe("book identity and links", () => {
  it("links every book by its numeric work id", () => {
    expect(bookPath(42)).toBe("/book/42");
    expect(bookPath({ id: 7 })).toBe("/book/7");
  });

  it("tells a work id from a legacy key the server resolves", () => {
    expect(parseBookParam("42")).toEqual({ kind: "id", id: 42 });
    expect(parseBookParam("OL1168007W")).toEqual({ kind: "legacy", key: "OL1168007W" });
    expect(parseBookParam("BCdeadbeef01")).toEqual({ kind: "legacy", key: "BCdeadbeef01" });
    expect(parseBookParam("ISBN9780316769488")).toEqual({ kind: "legacy", key: "ISBN9780316769488" });
    expect(parseBookParam("ISBN080442957x")).toEqual({ kind: "legacy", key: "ISBN080442957x" });
    expect(parseBookParam("GBzyTCAlFPjgYC")).toEqual({ kind: "legacy", key: "GBzyTCAlFPjgYC" });
    for (const junk of ["", "0", "042", "-1", "1.5", "99999999999999999", "OLW", "../x", "Circe"]) {
      expect(parseBookParam(junk)).toEqual({ kind: "invalid" });
    }
  });

  it("links Open Library works to Open Library, and nothing else", () => {
    expect(openLibraryUrl("/works/OL1W")).toBe("https://openlibrary.org/works/OL1W");
    expect(openLibraryUrl(null)).toBeNull();
    expect(openLibraryUrl("/works/BCdeadbeef01")).toBeNull();
    expect(extractIsbn("978-0-316-76948-8")).toBe("9780316769488");
    expect(extractIsbn("it")).toBeNull();
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

  it("counts the days in the given zone, the club's for a meeting", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-01T20:00:00Z"));
    // 23:30 UTC is already 01:30 the next day in Berlin.
    expect(relativeDay("2026-09-01T23:30:00Z", "UTC")).toBe("today");
    expect(relativeDay("2026-09-01T23:30:00Z", "Europe/Berlin")).toBe("tomorrow");
    // 08:00 UTC on the 3rd is still the 2nd in Honolulu.
    expect(relativeDay("2026-09-03T08:00:00Z", "UTC")).toBe("in 2 days");
    expect(relativeDay("2026-09-03T08:00:00Z", "Pacific/Honolulu")).toBe("tomorrow");
    expect(relativeDay("2026-09-01T23:30:00Z", "Not/AZone")).not.toBe("");
  });
});

describe("bookIdFromInput", () => {
  it("reads a book's id from a pasted link or a bare number", () => {
    expect(bookIdFromInput("42")).toBe(42);
    expect(bookIdFromInput(" /book/42 ")).toBe(42);
    expect(bookIdFromInput("https://bookclub.example/book/42#entry-3")).toBe(42);
    expect(bookIdFromInput("http://agents.lan:5173/book/7?x=1")).toBe(7);
    expect(bookIdFromInput("Circe")).toBeNull();
    expect(bookIdFromInput("/book/OL1W")).toBeNull();
    expect(bookIdFromInput("0")).toBeNull();
  });
});
