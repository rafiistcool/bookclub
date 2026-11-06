import { describe, expect, it } from "vitest";
import { detectByLines, detectByRegion, detectCover, MIN_CONFIDENCE } from "./detect";
import { isConvex, orderQuad, type PixelImage, type Quad } from "./geometry";
import { blurred, bookCover, cornerError, posedCover, ramp, random, renderScene, shadowed, solid, wood, type CoverPose, type Rgb } from "./scenes";

const CREAM: Rgb = [228, 218, 196];
const INK: Rgb = [30, 28, 40];
const TEAL: Rgb = [40, 120, 130];

type Background = (x: number, y: number) => Rgb;

interface Case {
  name: string;
  width: number;
  height: number;
  pose: CoverPose;
  cover: (u: number, v: number) => Rgb;
  /** The background, or a function of the cover's true corners that makes it (for a shadow). */
  background: Background | { around: (q: Quad) => Background };
  noise: number;
  /** Box blur radius after rendering: an out-of-focus photo. */
  blur?: number;
  /** Paints over the rendered photo, e.g. a finger. */
  paint?: (image: PixelImage) => void;
}


const cases: Case[] = [
  {
    name: "a plain bright cover, square on, on a dark table",
    width: 400,
    height: 300,
    pose: { width: 150, height: 225 },
    cover: solid(CREAM),
    background: solid([55, 50, 48]),
    noise: 6,
  },
  {
    name: "a printed cover turned by 12°",
    width: 300,
    height: 400,
    pose: { width: 180, height: 270, rotate: 12 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([70, 72, 80]),
    noise: 8,
  },
  {
    name: "a printed cover turned the other way by 25°",
    width: 400,
    height: 400,
    pose: { width: 170, height: 250, rotate: -25 },
    cover: bookCover([200, 60, 50], [250, 240, 220], [30, 30, 30]),
    background: solid([180, 180, 175]),
    noise: 8,
  },
  {
    name: "a cover tilted away by 30° (keystone)",
    width: 360,
    height: 480,
    pose: { width: 200, height: 300, tiltX: 30 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([60, 60, 66]),
    noise: 8,
  },
  {
    name: "a cover tilted sideways by 25°, turned by 10° and off centre",
    width: 480,
    height: 360,
    pose: { width: 170, height: 250, tiltY: 25, rotate: 10, dx: 30, dy: -10 },
    cover: bookCover([30, 50, 90], [240, 200, 80], [200, 200, 210]),
    background: solid([200, 190, 170]),
    noise: 8,
  },
  {
    name: "a dark cover on a light wooden table, tilted and turned",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, tiltX: 20, tiltY: -10, rotate: 8 },
    cover: bookCover([35, 35, 45], [220, 220, 220], [150, 40, 40]),
    background: wood([190, 150, 105]),
    noise: 6,
  },
  {
    name: "a bright cover on a wooden table",
    width: 400,
    height: 300,
    pose: { width: 160, height: 240, rotate: -6, tiltX: 15 },
    cover: bookCover(CREAM, INK, TEAL),
    background: wood([120, 85, 55]),
    noise: 6,
  },
  {
    name: "a cover whose left side has little contrast with the background",
    width: 400,
    height: 400,
    pose: { width: 200, height: 290, rotate: 5 },
    cover: solid([150, 150, 150]),
    background: ramp(128, 40, 400),
    noise: 5,
  },
  {
    name: "a large cover that nearly fills the picture",
    width: 384,
    height: 512,
    pose: { width: 320, height: 470, rotate: 3, tiltX: 10 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([40, 40, 40]),
    noise: 6,
  },
  {
    name: "a small cover",
    width: 400,
    height: 400,
    pose: { width: 135, height: 205, rotate: -15, dx: -40, dy: 30 },
    cover: bookCover([240, 200, 60], INK, [60, 140, 60]),
    background: solid([70, 70, 90]),
    noise: 8,
  },
  {
    name: "a cover filling a tenth of the photo, tilted and turned",
    width: 300,
    height: 400,
    pose: { width: 95, height: 140, rotate: 14, tiltX: 30, tiltY: -18, dy: 20 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([52, 55, 62]),
    noise: 8,
  },
  {
    name: "a two-tone cover held at a distance over a light table",
    width: 300,
    height: 400,
    pose: { width: 110, height: 165, rotate: 6, tiltX: 18, tiltY: 10 },
    // Cream above, mint below, a dark oval: the band's edge outlines a quad of its own.
    cover: (u, v) => ((u - 0.68) ** 2 / 0.02 + (v - 0.3) ** 2 / 0.012 < 1 ? [30, 30, 30] : v < 0.5 ? [236, 228, 212] : [176, 226, 196]),
    background: solid([214, 206, 190]),
    noise: 5,
  },
  {
    name: "a cover with a soft shadow on two sides",
    width: 400,
    height: 400,
    pose: { width: 200, height: 290, rotate: 5, tiltX: 15 },
    cover: bookCover(CREAM, INK, TEAL),
    background: { around: (q) => shadowed(q, [130, 125, 120]) },
    noise: 5,
  },
  {
    name: "a cover near a table edge and a dark band",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, rotate: 8, tiltX: 15 },
    cover: bookCover(CREAM, INK, TEAL),
    background: (x, y) => (y > 0.15 * x + 340 ? [200, 200, 200] : x < 30 ? [20, 20, 20] : [90, 70, 50]),
    noise: 5,
  },
  {
    name: "a cover next to another book",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, rotate: 8, dx: 30 },
    cover: bookCover(CREAM, INK, TEAL),
    background: (x, y) => (x < 70 && y > 40 && y < 360 ? [180, 40, 40] : [60, 60, 60]),
    noise: 5,
  },
  {
    name: "a cover out of focus",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, rotate: -12, tiltX: 20 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([70, 70, 70]),
    noise: 4,
    blur: 4,
  },
  {
    name: "a cover held by a finger across its bottom side",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, rotate: 6 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([60, 60, 60]),
    noise: 5,
    paint: (image) => {
      for (let y = 250; y < 400; y++) {
        for (let x = 180; x < 215; x++) image.data.set([210, 160, 140], (y * image.width + x) * 4);
      }
    },
  },
  {
    name: "a light cover on a barely darker table",
    width: 400,
    height: 400,
    pose: { width: 200, height: 290, rotate: 5, tiltX: 15 },
    cover: bookCover([235, 235, 232], INK, TEAL),
    background: solid([222, 222, 225]),
    noise: 4,
  },
  {
    name: "a cover in a grainy photo, turned and tilted",
    width: 400,
    height: 300,
    pose: { width: 160, height: 240, rotate: 20, tiltX: 25 },
    cover: bookCover(CREAM, INK, TEAL),
    background: solid([90, 90, 90]),
    noise: 25,
  },
  {
    name: "a cover tilted on both axes and turned by 22°",
    width: 400,
    height: 400,
    pose: { width: 190, height: 280, tiltX: -28, tiltY: 15, rotate: -22 },
    cover: bookCover([90, 30, 60], [240, 240, 240], [200, 160, 40]),
    background: wood([150, 120, 90]),
    noise: 10,
  },
];

function photo(c: Case, truth: Quad): PixelImage {
  const background = typeof c.background === "function" ? c.background : c.background.around(truth);
  let image = renderScene({ width: c.width, height: c.height, quad: truth, cover: c.cover, background, noise: c.noise, seed: 7 });
  if (c.blur) image = blurred(image, c.blur);
  c.paint?.(image);
  return image;
}

function diagonal(width: number, height: number): number {
  return Math.hypot(width, height);
}

describe("detectCover", () => {
  for (const c of cases) {
    it(`finds ${c.name}`, () => {
      const truth = orderQuad(posedCover(c.width, c.height, c.pose));
      const found = detectCover(photo(c, truth));
      expect(found).not.toBeNull();
      expect(isConvex(found!.quad)).toBe(true);
      expect(cornerError(found!.quad, truth)).toBeLessThan(0.03 * diagonal(c.width, c.height));
      expect(found!.confidence).toBeGreaterThanOrEqual(MIN_CONFIDENCE);
      expect(found!.confidence).toBeLessThanOrEqual(1);
    });
  }

  it("returns corners in the coordinates of a large picture", () => {
    const truth = orderQuad(posedCover(1200, 900, { width: 450, height: 660, rotate: 7, tiltX: 12 }));
    const image = renderScene({ width: 1200, height: 900, quad: truth, cover: bookCover(CREAM, INK, TEAL), background: solid([50, 50, 55]), noise: 6 });
    const found = detectCover(image);
    expect(found).not.toBeNull();
    expect(cornerError(found!.quad, truth)).toBeLessThan(0.03 * diagonal(1200, 900));
  });

  it("finds nothing in noise", () => {
    const rand = random(3);
    const width = 400;
    const height = 300;
    const data = new Uint8ClampedArray(width * height * 4);
    for (let i = 0; i < data.length; i += 4) {
      const v = rand() * 255;
      data[i] = v;
      data[i + 1] = rand() * 255;
      data[i + 2] = v;
      data[i + 3] = 255;
    }
    expect(detectCover({ data, width, height })).toBeNull();
  });

  it("finds nothing on a bare wooden table", () => {
    const table = renderScene({ width: 400, height: 300, quad: posedCover(400, 300, { width: 1, height: 1 }), cover: solid([0, 0, 0]), background: wood([140, 100, 70]), noise: 8 });
    expect(detectCover(table)).toBeNull();
  });

  it("finds nothing in an even picture or a soft gradient", () => {
    const flat = renderScene({ width: 320, height: 240, quad: posedCover(320, 240, { width: 1, height: 1 }), cover: solid([128, 128, 128]), background: solid([128, 128, 128]), noise: 3 });
    expect(detectCover(flat)).toBeNull();
    const gradient = renderScene({ width: 320, height: 240, quad: posedCover(320, 240, { width: 1, height: 1 }), cover: solid([0, 0, 0]), background: ramp(30, 220, 320), noise: 3 });
    expect(detectCover(gradient)).toBeNull();
  });

  it("keeps the corners inside the picture when the cover runs off it", () => {
    const truth = orderQuad(posedCover(300, 300, { width: 250, height: 380, rotate: 4 }));
    const image = renderScene({ width: 300, height: 300, quad: truth, cover: solid(CREAM), background: solid([40, 40, 40]), noise: 5 });
    const found = detectCover(image);
    // Only the left and right sides are in the picture: at best the visible part.
    for (const p of found?.quad ?? []) {
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.x).toBeLessThanOrEqual(300);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeLessThanOrEqual(300);
    }
  });

  it("prefers the cover over the picture printed on it", () => {
    const truth = orderQuad(posedCover(400, 400, { width: 220, height: 320 }));
    // A strong inner frame with a cover-like aspect, a weaker outline.
    const cover = (u: number, v: number): Rgb => (u > 0.12 && u < 0.88 && v > 0.15 && v < 0.85 && (u < 0.15 || u > 0.85 || v < 0.18 || v > 0.82) ? [0, 0, 0] : [140, 140, 140]);
    const image = renderScene({ width: 400, height: 400, quad: truth, cover, background: solid([95, 95, 95]), noise: 4 });
    const found = detectCover(image);
    expect(found).not.toBeNull();
    expect(cornerError(found!.quad, truth)).toBeLessThan(0.03 * diagonal(400, 400));
  });

  it("is quick on a 400 px picture", () => {
    const truth = orderQuad(posedCover(400, 300, { width: 160, height: 240, rotate: -6, tiltX: 15 }));
    const image = renderScene({ width: 400, height: 300, quad: truth, cover: bookCover(CREAM, INK, TEAL), background: wood([120, 85, 55]), noise: 6 });
    detectCover(image);
    const start = performance.now();
    for (let i = 0; i < 5; i++) detectCover(image);
    const each = (performance.now() - start) / 5;
    // About 20 ms here; the bound only catches a runaway search on a slow runner.
    expect(each).toBeLessThan(400);
  });

  it("returns null for a tiny picture", () => {
    const image: PixelImage = { data: new Uint8ClampedArray(8 * 8 * 4), width: 8, height: 8 };
    expect(detectCover(image)).toBeNull();
  });
});

describe("the two detectors", () => {
  const truth: Quad = orderQuad(posedCover(400, 300, { width: 150, height: 220, rotate: 10, tiltX: 15 }));

  it("the lines find a cover with sharp sides", () => {
    const image = renderScene({ width: 400, height: 300, quad: truth, cover: bookCover(CREAM, INK, TEAL), background: solid([60, 60, 60]), noise: 6 });
    const found = detectByLines(image);
    expect(found).not.toBeNull();
    expect(cornerError(orderQuad(found!.quad), truth)).toBeLessThan(0.03 * diagonal(400, 300));
  });

  it("the regions find a cover by its colour", () => {
    const image = renderScene({ width: 400, height: 300, quad: truth, cover: solid([200, 60, 60]), background: solid([60, 90, 60]), noise: 10 });
    const found = detectByRegion(image);
    expect(found).not.toBeNull();
    expect(cornerError(orderQuad(found!.quad), truth)).toBeLessThan(0.03 * diagonal(400, 300));
    expect(found!.confidence).toBeGreaterThan(0.6);
  });

  it("the regions give up on a picture of one colour", () => {
    const image = renderScene({ width: 200, height: 150, quad: truth, cover: solid([90, 90, 90]), background: solid([90, 90, 90]), noise: 4 });
    expect(detectByRegion(image)).toBeNull();
  });
});
