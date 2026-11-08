import { describe, expect, it } from "vitest";
import { nextBar } from "./topBar";

describe("the phone's top bar on scroll", () => {
  const shown = { hidden: false, anchor: 0 };

  it("always shows near the top", () => {
    expect(nextBar({ hidden: true, anchor: 300 }, 40, 56)).toEqual({ hidden: false, anchor: 40 });
  });

  it("hides going down and returns going up, past a small jitter", () => {
    const down = nextBar(shown, 200, 56);
    expect(down).toEqual({ hidden: true, anchor: 200 });
    expect(nextBar(down, 195, 56)).toBe(down);
    expect(nextBar(down, 180, 56)).toEqual({ hidden: false, anchor: 180 });
  });

  it("adds up a slow scroll: the anchor only moves with a decision", () => {
    let state = { hidden: false, anchor: 100 };
    for (const y of [103, 106, 109]) state = nextBar(state, y, 56);
    expect(state).toEqual({ hidden: true, anchor: 109 });
  });
});
