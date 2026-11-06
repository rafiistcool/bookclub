import { describe, expect, it } from "vitest";
import { calendarDay, formatClubDate, formatDay, formatRelative } from "./dates";

// Friday 6 Nov 2026, 12:00 UTC (13:00 in Berlin).
const NOW = Date.parse("2026-11-06T12:00:00Z");
const ago = (ms: number) => new Date(NOW - ms).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe("formatRelative", () => {
  it("says just now for under a minute and for a time ahead of the clock", () => {
    expect(formatRelative(ago(0), "Europe/Berlin", "de", NOW)).toBe("gerade eben");
    expect(formatRelative(ago(59_000), "Europe/Berlin", "en", NOW)).toBe("just now");
    expect(formatRelative(ago(-5 * MIN), "Europe/Berlin", "de", NOW)).toBe("gerade eben");
  });

  it("counts minutes, then hours on the same day", () => {
    expect(formatRelative(ago(5 * MIN), "Europe/Berlin", "de", NOW)).toBe("vor 5 Min.");
    expect(formatRelative(ago(5 * MIN), "Europe/Berlin", "en", NOW)).toBe("5 min ago");
    expect(formatRelative(ago(59 * MIN), "Europe/Berlin", "en", NOW)).toBe("59 min ago");
    expect(formatRelative(ago(3 * HOUR), "Europe/Berlin", "de", NOW)).toBe("vor 3 Std.");
    expect(formatRelative(ago(3 * HOUR), "Europe/Berlin", "en", NOW)).toBe("3 hr ago");
  });

  it("says yesterday for the day before in the zone, even an hour ago", () => {
    // 00:45 in Berlin; the post was written at 23:30 Berlin time, 75 minutes earlier.
    const now = Date.parse("2026-09-01T22:45:00Z");
    const iso = "2026-09-01T21:30:00Z";
    expect(formatRelative(iso, "Europe/Berlin", "de", now)).toBe("gestern");
    expect(formatRelative(iso, "Europe/Berlin", "en", now)).toBe("yesterday");
    // Still the same day in UTC and in New York.
    expect(formatRelative(iso, "UTC", "de", now)).toBe("vor 1 Std.");
    expect(formatRelative(iso, "America/New_York", "en", now)).toBe("1 hr ago");
  });

  it("puts the day boundary where the zone has it", () => {
    // 23:30 UTC on 1 Sep is already 2 Sep in Berlin.
    const iso = "2026-09-01T23:30:00Z";
    const now = Date.parse("2026-09-03T10:00:00Z");
    expect(formatRelative(iso, "Europe/Berlin", "de", now)).toBe("gestern");
    expect(formatRelative(iso, "UTC", "de", now)).toBe("Dienstag");
    expect(formatRelative(iso, "UTC", "en", now)).toBe("Tuesday");
  });

  it("names the weekday from 2 to 6 days back, never the day before yesterday", () => {
    expect(formatRelative(ago(2 * DAY), "Europe/Berlin", "de", NOW)).toBe("Mittwoch");
    expect(formatRelative(ago(2 * DAY), "Europe/Berlin", "en", NOW)).toBe("Wednesday");
    expect(formatRelative(ago(6 * DAY), "Europe/Berlin", "de", NOW)).toBe("Samstag");
    expect(formatRelative(ago(6 * DAY), "Europe/Berlin", "de", NOW)).not.toContain("vorgestern");
  });

  it("gives the day and month this year, and the year before it", () => {
    expect(formatRelative("2026-10-12T10:00:00Z", "Europe/Berlin", "de", NOW)).toBe("12. Okt.");
    expect(formatRelative("2026-10-12T10:00:00Z", "Europe/Berlin", "en", NOW)).toBe("12 Oct");
    expect(formatRelative("2025-10-12T10:00:00Z", "Europe/Berlin", "de", NOW)).toBe("12. Okt. 2025");
    expect(formatRelative("2025-10-12T10:00:00Z", "Europe/Berlin", "en", NOW)).toBe("12 Oct 2025");
  });

  it("decides the year in the zone too", () => {
    // New Year's Eve 23:30 in UTC is 1 Jan 2027 in Berlin.
    const iso = "2026-12-31T23:30:00Z";
    const now = Date.parse("2027-01-20T12:00:00Z");
    expect(formatRelative(iso, "Europe/Berlin", "de", now)).toBe("1. Jan.");
    expect(formatRelative(iso, "UTC", "de", now)).toBe("31. Dez. 2026");
  });

  it("gives nothing for a missing or broken time, and survives an unknown zone", () => {
    expect(formatRelative(null, "UTC", "en", NOW)).toBe("");
    expect(formatRelative("not a date", "UTC", "en", NOW)).toBe("");
    expect(() => formatRelative(ago(3 * DAY), "Mars/Olympus", "en", NOW)).not.toThrow();
    expect(formatRelative(ago(5 * MIN), "Mars/Olympus", "en", NOW)).toBe("5 min ago");
    expect(formatRelative("2025-10-12T10:00:00Z", "", "en", NOW)).toBe("12 Oct 2025");
  });

  it("uses the real clock without a now", () => {
    expect(formatRelative(new Date().toISOString(), "UTC", "en")).toBe("just now");
  });
});

describe("calendarDay", () => {
  it("returns the zone's day as a UTC midnight", () => {
    const date = new Date("2026-09-01T23:30:00Z");
    expect(calendarDay(date, "Europe/Berlin")).toBe(Date.UTC(2026, 8, 2));
    expect(calendarDay(date, "UTC")).toBe(Date.UTC(2026, 8, 1));
  });
});

describe("absolute dates stay", () => {
  it("formats the exact time with the zone and a day without it", () => {
    expect(formatClubDate("2026-11-06T17:21:00Z", "Europe/Berlin", "de")).toBe("Fr., 6. Nov. 2026, 18:21 MEZ");
    expect(formatDay("2026-03-12T10:00:00Z", "Europe/Berlin", "en")).toBe("12 Mar 2026");
  });
});
