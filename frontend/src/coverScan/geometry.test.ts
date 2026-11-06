import { describe, expect, it } from "vitest";
import { clampQuad, insetQuad, isConvex, orderQuad, outputSize, quadArea, quadSides, scaleQuad, type Point, type Quad } from "./geometry";

function rotated(cx: number, cy: number, w: number, h: number, degrees: number): Quad {
  const a = (degrees * Math.PI) / 180;
  const corners: [number, number][] = [
    [-w / 2, -h / 2],
    [w / 2, -h / 2],
    [w / 2, h / 2],
    [-w / 2, h / 2],
  ];
  return corners.map(([x, y]) => ({ x: cx + x * Math.cos(a) - y * Math.sin(a), y: cy + x * Math.sin(a) + y * Math.cos(a) })) as Quad;
}

function close(a: Point[], b: Point[]): void {
  expect(a).toHaveLength(b.length);
  a.forEach((p, i) => {
    expect(p.x).toBeCloseTo(b[i].x, 6);
    expect(p.y).toBeCloseTo(b[i].y, 6);
  });
}

describe("orderQuad", () => {
  it("puts any order of a rectangle's corners into top-left, top-right, bottom-right, bottom-left", () => {
    const truth = rotated(200, 150, 120, 180, 0);
    const shuffles = [
      [2, 0, 3, 1],
      [3, 2, 1, 0],
      [1, 3, 0, 2],
      [0, 1, 2, 3],
    ];
    for (const order of shuffles) close(orderQuad(order.map((i) => truth[i])), truth);
  });

  it("keeps the order of a turned cover up to 40° either way", () => {
    for (const degrees of [-40, -25, -10, 5, 18, 25, 40]) {
      const truth = rotated(200, 200, 130, 200, degrees);
      close(orderQuad([truth[3], truth[1], truth[0], truth[2]]), truth);
    }
  });

  it("orders a keystone quad", () => {
    const truth: Quad = [
      { x: 120, y: 60 },
      { x: 280, y: 70 },
      { x: 330, y: 390 },
      { x: 70, y: 380 },
    ];
    close(orderQuad([truth[2], truth[0], truth[3], truth[1]]), truth);
  });

  it("copies the points instead of returning them", () => {
    const points = rotated(50, 50, 20, 30, 0);
    const out = orderQuad(points);
    out[0].x = -1;
    expect(points[0].x).not.toBe(-1);
  });

  it("refuses anything but four points", () => {
    expect(() => orderQuad([{ x: 0, y: 0 }])).toThrow(RangeError);
    expect(() => orderQuad([...rotated(0, 0, 1, 1, 0), { x: 3, y: 3 }])).toThrow(RangeError);
  });
});

describe("insetQuad", () => {
  it("insets each side by 8 % by default", () => {
    expect(insetQuad(1000, 500)).toEqual([
      { x: 80, y: 40 },
      { x: 920, y: 40 },
      { x: 920, y: 460 },
      { x: 80, y: 460 },
    ]);
  });

  it("takes another inset", () => {
    expect(insetQuad(200, 100, 0)).toEqual([
      { x: 0, y: 0 },
      { x: 200, y: 0 },
      { x: 200, y: 100 },
      { x: 0, y: 100 },
    ]);
    expect(isConvex(insetQuad(640, 480, 0.2))).toBe(true);
  });
});

describe("isConvex", () => {
  it("accepts rectangles, turned covers and keystones in Quad order", () => {
    expect(isConvex(insetQuad(100, 100))).toBe(true);
    expect(isConvex(rotated(100, 100, 60, 90, 30))).toBe(true);
    expect(
      isConvex([
        { x: 40, y: 10 },
        { x: 60, y: 10 },
        { x: 100, y: 100 },
        { x: 0, y: 100 },
      ]),
    ).toBe(true);
  });

  it("refuses a crossed quad", () => {
    const [tl, tr, br, bl] = insetQuad(100, 100);
    expect(isConvex([tl, tr, bl, br])).toBe(false);
    expect(isConvex([tl, br, tr, bl])).toBe(false);
  });

  it("refuses a dent", () => {
    expect(
      isConvex([
        { x: 0, y: 0 },
        { x: 100, y: 0 },
        { x: 40, y: 40 },
        { x: 0, y: 100 },
      ]),
    ).toBe(false);
  });

  it("refuses a mirrored (counter-clockwise) order", () => {
    const [tl, tr, br, bl] = insetQuad(100, 100);
    expect(isConvex([tl, bl, br, tr])).toBe(false);
  });

  it("refuses three corners on one line, a collapsed quad and NaN", () => {
    expect(
      isConvex([
        { x: 0, y: 0 },
        { x: 50, y: 0 },
        { x: 100, y: 0 },
        { x: 0, y: 100 },
      ]),
    ).toBe(false);
    const p = { x: 5, y: 5 };
    expect(isConvex([p, p, p, p])).toBe(false);
    expect(isConvex([{ x: Number.NaN, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }, { x: 0, y: 1 }])).toBe(false);
  });
});

describe("quadArea and quadSides", () => {
  it("measures a rectangle", () => {
    const q = insetQuad(300, 400, 0);
    expect(quadArea(q)).toBe(120000);
    expect(quadSides(q)).toEqual({ width: 300, height: 400 });
  });

  it("averages the opposite sides of a keystone", () => {
    const q: Quad = [
      { x: 100, y: 0 },
      { x: 200, y: 0 },
      { x: 250, y: 300 },
      { x: 50, y: 300 },
    ];
    expect(quadSides(q).width).toBeCloseTo(150, 6);
    expect(quadSides(q).height).toBeCloseTo(Math.hypot(50, 300), 6);
  });
});

describe("outputSize", () => {
  it("keeps the size of an upright cover", () => {
    expect(outputSize(insetQuad(300, 450, 0))).toEqual({ width: 300, height: 450 });
  });

  it("caps the long side at 1600 px, or another maximum", () => {
    expect(outputSize(insetQuad(2000, 3000, 0))).toEqual({ width: 1067, height: 1600 });
    expect(outputSize(insetQuad(2000, 3000, 0), 900)).toEqual({ width: 600, height: 900 });
  });

  it("measures a turned cover by its sides, not its bounding box", () => {
    const size = outputSize(rotated(500, 500, 400, 600, 20));
    expect(size).toEqual({ width: 400, height: 600 });
  });

  it("clamps a wide quad to a square", () => {
    expect(outputSize(insetQuad(1000, 500, 0))).toEqual({ width: 1000, height: 1000 });
  });

  it("clamps a narrow quad to 0.4", () => {
    expect(outputSize(insetQuad(100, 1000, 0))).toEqual({ width: 400, height: 1000 });
  });

  it("never returns less than 1 px", () => {
    const p = { x: 3, y: 3 };
    expect(outputSize([p, p, p, p])).toEqual({ width: 1, height: 1 });
  });
});

describe("scaleQuad and clampQuad", () => {
  it("scales every corner", () => {
    expect(scaleQuad(insetQuad(100, 50, 0), 4)).toEqual(insetQuad(400, 200, 0));
  });

  it("pulls corners back into the picture", () => {
    const q: Quad = [
      { x: -5, y: -1 },
      { x: 120, y: 3 },
      { x: 99, y: 80 },
      { x: 2, y: 70 },
    ];
    expect(clampQuad(q, 100, 75)).toEqual([
      { x: 0, y: 0 },
      { x: 100, y: 3 },
      { x: 99, y: 75 },
      { x: 2, y: 70 },
    ]);
  });
});
