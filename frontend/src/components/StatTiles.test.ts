import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import StatTiles from "./StatTiles.vue";

const tiles = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ key: `k${i}`, label: `Label ${i}`, value: String(i) }));

describe("StatTiles", () => {
  it("sets each figure over its label, in a list of terms and values", () => {
    const wrapper = mount(StatTiles, { props: { items: tiles(2) } });
    const tile = wrapper.get('[data-stat="k1"]');
    expect(tile.get("dt").text()).toBe("Label 1");
    expect(tile.get("dd").text()).toBe("1");
  });

  it("picks columns that fill every row", () => {
    const style = (n: number) => mount(StatTiles, { props: { items: tiles(n) } }).get(".stat-tiles").attributes("style");
    // Six figures: 3 by 2 everywhere. Four: 2 by 2 on a phone, one row when wide.
    expect(style(6)).toContain("--cols: 3");
    expect(style(6)).toContain("--cols-wide: 3");
    expect(style(4)).toContain("--cols: 2");
    expect(style(4)).toContain("--cols-wide: 4");
  });
});
