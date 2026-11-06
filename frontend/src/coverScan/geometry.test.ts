import { describe, expect, it } from "vitest";
import { clampQuad, coverAspect, insetQuad, isConvex, MAX_ASPECT, MIN_ASPECT, orderQuad, outputSize, quadArea, quadSides, rotateQuad, scaleQuad, type PixelImage, type Point, type Quad } from "./geometry";
import { posedCover, random } from "./scenes";
import { warpQuad } from "./warp";

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

  it("keeps a landscape cover landscape, its long side on the width", () => {
    expect(outputSize(insetQuad(1000, 500, 0))).toEqual({ width: 1000, height: 500 });
    expect(outputSize(insetQuad(3000, 2000, 0))).toEqual({ width: 1600, height: 1067 });
  });

  it("clamps a narrow quad to 0.4 and a flat one to 2.5", () => {
    expect(MAX_ASPECT).toBeCloseTo(1 / MIN_ASPECT, 12);
    expect(outputSize(insetQuad(100, 1000, 0))).toEqual({ width: 400, height: 1000 });
    expect(outputSize(insetQuad(1000, 100, 0))).toEqual({ width: 1000, height: 400 });
  });

  it("takes the cover's own aspect when given one", () => {
    // A cover tilted away looks 300 × 200; it is 2:3.
    expect(outputSize(insetQuad(300, 200, 0), 1600, 2 / 3)).toEqual({ width: 200, height: 300 });
    expect(outputSize(insetQuad(300, 450, 0), 1600, 1.5)).toEqual({ width: 450, height: 300 });
    expect(outputSize(insetQuad(300, 450, 0), 1600, 9)).toEqual({ width: 450, height: 180 });
    // Nonsense falls back to the quad's own.
    expect(outputSize(insetQuad(300, 450, 0), 1600, Number.NaN)).toEqual({ width: 300, height: 450 });
    expect(outputSize(insetQuad(300, 450, 0), 1600, 0)).toEqual({ width: 300, height: 450 });
  });

  it("never returns less than 1 px", () => {
    const p = { x: 3, y: 3 };
    expect(outputSize([p, p, p, p])).toEqual({ width: 1, height: 1 });
  });
});

describe("coverAspect", () => {
  const TRUE = 200 / 300;
  // posedCover's camera has a focal length of 1.6 × the cover's long side;
  // in a 450 × 600 photo that is a phone's 0.8 × the photo's long side.
  const W = 450;
  const H = 600;
  const posed = (pose: { tiltX?: number; tiltY?: number; rotate?: number }, w = W, h = H, width = 200, height = 300) =>
    orderQuad(posedCover(w, h, { width, height, ...pose }));
  const photographed = (q: Quad) => quadSides(q).width / quadSides(q).height;

  it("undoes the foreshortening of a cover tilted away or sideways", () => {
    for (const pose of [{ tiltX: 30 }, { tiltX: 45 }, { tiltY: 30 }, { tiltX: -35, rotate: 12 }]) {
      const q = posed(pose);
      expect(Math.abs(photographed(q) / TRUE - 1)).toBeGreaterThan(0.1);
      expect(Math.abs(coverAspect(q, W, H) / TRUE - 1)).toBeLessThan(0.03);
    }
  });

  it("measures the focal length when the cover is tilted on both axes", () => {
    // A 360 × 480 photo: the assumed focal length is wrong by a quarter, the measured one is not.
    for (const pose of [{ tiltX: 20, tiltY: 15 }, { tiltX: -28, tiltY: 15, rotate: -22 }]) {
      expect(Math.abs(coverAspect(posed(pose, 360, 480), 360, 480) / TRUE - 1)).toBeLessThan(0.03);
    }
  });

  it("is still closer than the photographed ratio when the assumed focal length is off", () => {
    for (const [w, h] of [
      [360, 480],
      [600, 800],
    ]) {
      for (const pose of [{ tiltX: 30 }, { tiltX: 45 }]) {
        const q = posed(pose, w, h);
        expect(Math.abs(coverAspect(q, w, h) / TRUE - 1)).toBeLessThan(Math.abs(photographed(q) / TRUE - 1));
      }
    }
  });

  it("stays within 10 % when the corners are off by 1 % of the diagonal", () => {
    const rand = random(11);
    for (const pose of [{ tiltX: 30 }, { tiltX: 45 }, { tiltY: 30 }, { tiltX: 20, tiltY: 15 }, {}]) {
      const q = posed(pose);
      const step = 0.01 * Math.hypot(quadSides(q).width, quadSides(q).height);
      for (let k = 0; k < 25; k++) {
        const jittered = q.map((p) => {
          const a = rand() * 2 * Math.PI;
          return { x: p.x + step * Math.cos(a), y: p.y + step * Math.sin(a) };
        }) as Quad;
        expect(Math.abs(coverAspect(jittered, W, H) / TRUE - 1)).toBeLessThan(0.1);
      }
    }
  });

  it("finds a landscape cover's aspect, and the inverse for the quad turned", () => {
    const q = posed({ tiltX: 30 }, W, H, 300, 200);
    expect(Math.abs(coverAspect(q, W, H) / 1.5 - 1)).toBeLessThan(0.03);
    const portrait = posed({ tiltX: 30 });
    expect(coverAspect(rotateQuad(portrait, 1), W, H) * coverAspect(portrait, W, H)).toBeCloseTo(1, 6);
  });

  it("keeps the photographed ratio of a quad without perspective", () => {
    expect(coverAspect(insetQuad(300, 450, 0), 1000, 1000)).toBeCloseTo(300 / 450, 9);
    const turned: Quad = [
      { x: 500, y: 200 },
      { x: 800, y: 500 },
      { x: 500, y: 800 },
      { x: 200, y: 500 },
    ];
    expect(coverAspect(turned, 1000, 1000)).toBeCloseTo(1, 9);
  });

  it("never corrects by more than a factor of 1.6, and falls back on nonsense", () => {
    // A keystone that would mean a tilt of about 70°.
    const steep: Quad = [
      { x: 480, y: 100 },
      { x: 520, y: 100 },
      { x: 900, y: 300 },
      { x: 100, y: 300 },
    ];
    const ratio = photographed(steep);
    const aspect = coverAspect(steep, 1000, 1000);
    expect(aspect).toBeLessThanOrEqual(ratio * 1.6 + 1e-9);
    expect(aspect).toBeGreaterThanOrEqual(ratio / 1.6 - 1e-9);
    const [tl, tr, br, bl] = insetQuad(300, 450, 0);
    expect(coverAspect([tl, br, tr, bl], 1000, 1000)).toBeCloseTo(photographed([tl, br, tr, bl]), 9);
    expect(coverAspect(insetQuad(300, 450, 0), 0, 0)).toBeCloseTo(300 / 450, 9);
    const p = { x: 3, y: 3 };
    expect(coverAspect([p, p, p, p], 100, 100)).toBe(1);
  });
});

describe("rotateQuad", () => {
  const q = insetQuad(300, 400, 0);

  it("moves the corners on by quarter turns, either way", () => {
    expect(rotateQuad(q, 0)).toEqual(q);
    expect(rotateQuad(q, 1)).toEqual([q[3], q[0], q[1], q[2]]);
    expect(rotateQuad(q, 2)).toEqual([q[2], q[3], q[0], q[1]]);
    expect(rotateQuad(q, 3)).toEqual(rotateQuad(q, -1));
    expect(rotateQuad(q, 5)).toEqual(rotateQuad(q, 1));
    expect(isConvex(rotateQuad(q, 1))).toBe(true);
    const out = rotateQuad(q, 1);
    out[0].x = -1;
    expect(q[3].x).not.toBe(-1);
  });

  it("makes the warp turn the picture clockwise by 90° per turn", () => {
    // A 3 × 2 picture with a different value in every pixel.
    const data = new Uint8ClampedArray(3 * 2 * 4);
    for (let i = 0; i < 6; i++) data.set([i * 40, 0, 0, 255], i * 4);
    const source: PixelImage = { data, width: 3, height: 2 };
    const red = (image: ImageData) => Array.from({ length: image.width * image.height }, (_, i) => image.data[i * 4]);
    const whole = insetQuad(3, 2, 0);
    expect(red(warpQuad(source, whole, 3, 2))).toEqual([0, 40, 80, 120, 160, 200]);
    // Clockwise: the left column (0 above 120) becomes the top row, read right to left.
    expect(red(warpQuad(source, rotateQuad(whole, 1), 2, 3))).toEqual([120, 0, 160, 40, 200, 80]);
    expect(red(warpQuad(source, rotateQuad(whole, 2), 3, 2))).toEqual([200, 160, 120, 80, 40, 0]);
    expect(red(warpQuad(source, rotateQuad(whole, -1), 2, 3))).toEqual([80, 200, 40, 160, 0, 120]);
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
