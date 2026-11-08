import { describe, expect, it } from "vitest";
import { pageOfPercent, pagePercent, positionOf, positionPatch, positionSaved } from "./progress";

describe("pagePercent", () => {
  it("rounds like the server, 100 only on the last page and 1 from the first", () => {
    expect(pagePercent(207, 334)).toBe(62);
    expect(pagePercent(208, 334)).toBe(62);
    expect(pagePercent(218, 334)).toBe(65);
    expect(pagePercent(1, 334)).toBe(1);
    expect(pagePercent(333, 334)).toBe(99);
    expect(pagePercent(334, 334)).toBe(100);
    expect(pagePercent(0, 334)).toBe(0);
    expect(pagePercent(999, 334)).toBe(100);
  });

  it("derives the page an older save in percent stands for", () => {
    expect(pageOfPercent(50, 300)).toBe(150);
    expect(pageOfPercent(35, 50)).toBe(18);
  });
});

describe("positions", () => {
  it("saves the page when there is one, else the percent", () => {
    expect(positionPatch({ progress: 62, page: 208 })).toEqual({ current_page: 208 });
    expect(positionPatch({ progress: 40, page: null })).toEqual({ progress: 40 });
  });

  it("reads an entry or a reader row, and says what was saved", () => {
    expect(positionOf({ progress: 62, current_page: 208 })).toEqual({ progress: 62, page: 208 });
    expect(positionOf({ progress: null })).toEqual({ progress: 0, page: null });
    expect(positionOf(null)).toEqual({ progress: 0, page: null });
    expect(positionSaved({ progress: 62, page: 208 })).toBe("Progress: p. 208 · 62%");
    expect(positionSaved({ progress: 40, page: null })).toBe("Progress: 40%");
  });
});
