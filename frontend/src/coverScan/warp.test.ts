import { describe, expect, it } from "vitest";
import { insetQuad, type PixelImage, type Quad } from "./geometry";
import { renderScene, type Rgb } from "./scenes";
import { applyHomography, homography, warpQuad } from "./warp";

const keystone: Quad = [
  { x: 130.5, y: 70.25 },
  { x: 290, y: 85 },
  { x: 340, y: 410 },
  { x: 60, y: 380.75 },
];

function pixel(image: PixelImage, x: number, y: number): number[] {
  const i = (y * image.width + x) * 4;
  return Array.from(image.data.slice(i, i + 4));
}

describe("homography", () => {
  it("maps each corner onto its partner", () => {
    const to = insetQuad(400, 600, 0);
    const h = homography(keystone, to);
    expect(h).toHaveLength(9);
    expect(h[8]).toBe(1);
    keystone.forEach((p, i) => {
      const q = applyHomography(h, p);
      expect(q.x).toBeCloseTo(to[i].x, 6);
      expect(q.y).toBeCloseTo(to[i].y, 6);
    });
  });

  it("is the identity for a quad onto itself", () => {
    const h = homography(keystone, keystone);
    [1, 0, 0, 0, 1, 0, 0, 0, 1].forEach((v, i) => expect(h[i]).toBeCloseTo(v, 9));
  });

  it("maps the centre of a rectangle onto the crossing of the quad's diagonals", () => {
    const h = homography(insetQuad(100, 100, 0), keystone);
    const centre = applyHomography(h, { x: 50, y: 50 });
    // The diagonals TL–BR and TR–BL meet where both lines pass.
    const [a, b, c, d] = keystone;
    const cross = (p: { x: number; y: number }, q: { x: number; y: number }, r: { x: number; y: number }) => (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    expect(cross(a, c, centre)).toBeCloseTo(0, 6);
    expect(cross(b, d, centre)).toBeCloseTo(0, 6);
  });

  it("is inverted by the homography the other way", () => {
    const to = insetQuad(800, 1200, 0);
    const there = homography(keystone, to);
    const back = homography(to, keystone);
    for (const p of [
      { x: 150, y: 200 },
      { x: 300, y: 380 },
      { x: 61, y: 380 },
    ]) {
      const q = applyHomography(back, applyHomography(there, p));
      expect(q.x).toBeCloseTo(p.x, 6);
      expect(q.y).toBeCloseTo(p.y, 6);
    }
  });

  it("works on photo-sized coordinates", () => {
    const big: Quad = keystone.map((p) => ({ x: p.x * 12, y: p.y * 12 })) as Quad;
    const to = insetQuad(1067, 1600, 0);
    const h = homography(big, to);
    big.forEach((p, i) => {
      const q = applyHomography(h, p);
      expect(q.x).toBeCloseTo(to[i].x, 5);
      expect(q.y).toBeCloseTo(to[i].y, 5);
    });
  });

  it("refuses three corners on one line", () => {
    const line: Quad = [
      { x: 0, y: 0 },
      { x: 50, y: 0 },
      { x: 100, y: 0 },
      { x: 0, y: 100 },
    ];
    expect(() => homography(line, insetQuad(10, 10, 0))).toThrow(RangeError);
    expect(() => homography(insetQuad(10, 10, 0), line)).toThrow(RangeError);
  });
});

describe("warpQuad", () => {
  function gradient(width: number, height: number): PixelImage {
    const data = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) data.set([x * 3, y * 5, (x + y) % 256, 255], (y * width + x) * 4);
    }
    return { data, width, height };
  }

  it("copies the picture unchanged when the quad is the whole picture", () => {
    const source = gradient(40, 30);
    const out = warpQuad(source, insetQuad(40, 30, 0), 40, 30);
    expect(out.width).toBe(40);
    expect(out.height).toBe(30);
    expect(Array.from(out.data)).toEqual(Array.from(source.data));
  });

  it("crops an axis-aligned quad", () => {
    const source = gradient(60, 50);
    const quad: Quad = [
      { x: 10, y: 5 },
      { x: 40, y: 5 },
      { x: 40, y: 45 },
      { x: 10, y: 45 },
    ];
    const out = warpQuad(source, quad, 30, 40);
    expect(pixel(out, 0, 0)).toEqual(pixel(source, 10, 5));
    expect(pixel(out, 29, 39)).toEqual(pixel(source, 39, 44));
    expect(pixel(out, 12, 20)).toEqual(pixel(source, 22, 25));
  });

  it("flattens a cover photographed at an angle back into its pattern", () => {
    // A 4 × 6 board of coloured squares, drawn through the keystone.
    const colours: Rgb[] = [
      [230, 40, 40],
      [40, 200, 60],
      [40, 60, 220],
      [240, 220, 40],
      [20, 20, 20],
      [250, 250, 250],
    ];
    const cell = (u: number, v: number) => Math.min(3, Math.floor(u * 4)) + 4 * Math.min(5, Math.floor(v * 6));
    const cover = (u: number, v: number): Rgb => colours[cell(u, v) % colours.length];
    const photo = renderScene({ width: 400, height: 450, quad: keystone, cover, background: () => [128, 128, 128] });
    const out = warpQuad(photo, keystone, 200, 300);
    expect(out.width).toBe(200);
    expect(out.height).toBe(300);
    for (let row = 0; row < 6; row++) {
      for (let col = 0; col < 4; col++) {
        // The centre of each square, well away from its borders.
        const got = pixel(out, col * 50 + 25, row * 50 + 25);
        const want = colours[(col + 4 * row) % colours.length];
        for (let c = 0; c < 3; c++) expect(Math.abs(got[c] - want[c])).toBeLessThanOrEqual(3);
        expect(got[3]).toBe(255);
      }
    }
    // Next to the corners the background must not leak in.
    for (const [x, y] of [
      [1, 1],
      [198, 1],
      [198, 298],
      [1, 298],
    ]) {
      const got = pixel(out, x, y);
      expect(got.slice(0, 3)).not.toEqual([128, 128, 128]);
    }
  });

  it("interpolates between pixels", () => {
    const data = new Uint8ClampedArray([0, 0, 0, 255, 200, 100, 52, 255]);
    const source = { data, width: 2, height: 1 };
    // Output pixel 1 of 4 samples the source at x = 0.75: a quarter of the way between the two pixel centres.
    const out = warpQuad(source, insetQuad(2, 1, 0), 4, 1);
    expect(pixel(out, 0, 0)).toEqual([0, 0, 0, 255]);
    expect(pixel(out, 1, 0)).toEqual([50, 25, 13, 255]);
    expect(pixel(out, 3, 0)).toEqual([200, 100, 52, 255]);
  });

  it("takes the nearest edge pixel outside the source and makes the output opaque", () => {
    const data = new Uint8ClampedArray([10, 20, 30, 0]);
    const out = warpQuad({ data, width: 1, height: 1 }, insetQuad(1, 1, -1), 3, 3);
    for (let y = 0; y < 3; y++) for (let x = 0; x < 3; x++) expect(pixel(out, x, y)).toEqual([10, 20, 30, 255]);
  });

  it("rounds the output size and keeps at least 1 px", () => {
    const out = warpQuad({ data: new Uint8ClampedArray(4), width: 1, height: 1 }, insetQuad(1, 1, 0), 0.2, 2.6);
    expect(out.width).toBe(1);
    expect(out.height).toBe(3);
    expect(out.data).toHaveLength(12);
  });
});
