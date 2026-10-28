import { describe, expect, it } from "vitest";
import { formatCount, formatRating, monthName, monthSeries } from "./stats";

describe("year in books helpers", () => {
  it("fills every month of the year, ignoring other years", () => {
    const series = monthSeries(2026, [
      { month: "2026-03", finished: 2 },
      { month: "2026-11", finished: 1 },
      { month: "2025-03", finished: 9 },
    ]);
    expect(series).toHaveLength(12);
    expect(series[0]).toEqual({ month: "2026-01", finished: 0 });
    expect(series[2]).toEqual({ month: "2026-03", finished: 2 });
    expect(series[10]).toEqual({ month: "2026-11", finished: 1 });
    expect(series.reduce((sum, row) => sum + row.finished, 0)).toBe(3);
  });

  it("names months and numbers in the reader's language", () => {
    expect(monthName("2026-03", "en")).toBe("Mar");
    expect(monthName("2026-03", "de", "long")).toBe("März");
    expect(formatCount(12480, "en")).toBe("12,480");
    expect(formatCount(12480, "de")).toBe("12.480");
    expect(formatRating(4.25, "en")).toBe("4.3");
    expect(formatRating(4, "de")).toBe("4,0");
    expect(formatRating(null, "en")).toBe("–");
  });
});
