import { describe, expect, it } from "vitest";
import {
  STATUS_NEXT,
  avatarColor,
  countdown,
  coverUrl,
  excerpt,
  initials,
  monogram,
  plural,
} from "./constants";

describe("countdown", () => {
  const now = new Date("2026-09-01T12:00:00Z");

  it("returns null for missing or invalid dates", () => {
    expect(countdown(null, now)).toBeNull();
    expect(countdown("not a date", now)).toBeNull();
  });

  it("formats future and past distances", () => {
    expect(countdown("2026-09-01T12:00:30Z", now)).toMatchObject({ label: "now", soon: true, past: false });
    expect(countdown("2026-09-01T12:25:00Z", now)?.label).toBe("in 25 min");
    expect(countdown("2026-09-01T15:00:00Z", now)?.label).toBe("in 3 h");
    expect(countdown("2026-09-02T12:00:00Z", now)?.label).toBe("tomorrow");
    expect(countdown("2026-09-10T12:00:00Z", now)).toMatchObject({ label: "in 9 days", soon: false });
    expect(countdown("2026-08-31T12:00:00Z", now)).toMatchObject({ label: "yesterday", past: true });
    expect(countdown("2026-08-20T12:00:00Z", now)?.label).toBe("12 days ago");
  });

  it("marks anything inside 48 hours as soon", () => {
    expect(countdown("2026-09-03T11:00:00Z", now)?.soon).toBe(true);
    expect(countdown("2026-09-03T13:00:00Z", now)?.soon).toBe(false);
  });
});

describe("avatars", () => {
  it("is deterministic per username and differs between names", () => {
    expect(avatarColor("rafi")).toBe(avatarColor("rafi"));
    expect(avatarColor("rafi")).toMatch(/^hsl\(\d+ 42% 44%\)$/);
    const colors = new Set(["rafi", "mara", "tom", "ada", "grace"].map(avatarColor));
    expect(colors.size).toBeGreaterThan(2);
  });

  it("builds initials from the first two alphanumerics", () => {
    expect(initials("rafi")).toBe("RA");
    expect(initials("_x_")).toBe("X");
    expect(initials("")).toBe("?");
  });
});

describe("helpers", () => {
  it("suggests the likely next status", () => {
    expect(STATUS_NEXT.want_to_read).toBe("currently_reading");
    expect(STATUS_NEXT.currently_reading).toBe("finished");
    expect(STATUS_NEXT.finished).toBe("want_to_read");
  });

  it("builds cover urls and monograms", () => {
    expect(coverUrl(123, "M")).toBe("https://covers.openlibrary.org/b/id/123-M.jpg");
    expect(coverUrl(null)).toBeNull();
    expect(monogram("  dune")).toBe("D");
    expect(monogram("")).toBe("?");
  });

  it("pluralises and trims excerpts", () => {
    expect(plural(1, "vote")).toBe("1 vote");
    expect(plural(2, "vote")).toBe("2 votes");
    expect(excerpt("a  b\n c", 10)).toBe("a b c");
    expect(excerpt("x".repeat(30), 10)).toHaveLength(10);
    expect(excerpt("x".repeat(30), 10).endsWith("…")).toBe(true);
  });
});
