/**
 * Synthetic photos of book covers for the cover-scan tests: a cover pattern
 * drawn through a known perspective onto a background, with noise. Test-only;
 * nothing in the app imports it.
 */

import type { PixelImage, Point, Quad } from "./geometry";
import { applyHomography, homography } from "./warp";

export type Rgb = [number, number, number];

/** A seeded PRNG (mulberry32), so every scene is the same on every run. */
export function random(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function gaussian(rand: () => number): number {
  const u = Math.max(rand(), 1e-12);
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * rand());
}

export interface CoverPose {
  /** The cover's size in pixels before the perspective. */
  width: number;
  height: number;
  /** Turn in the picture's plane, degrees, clockwise. */
  rotate?: number;
  /** Tilt towards the camera about the horizontal and the vertical axis, degrees. */
  tiltX?: number;
  tiltY?: number;
  /** Offset of the cover's centre from the picture's centre. */
  dx?: number;
  dy?: number;
}

/** The corners of a cover seen by a pinhole camera, in `Quad` order. */
export function posedCover(pictureWidth: number, pictureHeight: number, pose: CoverPose): Quad {
  const d = 1.6 * Math.max(pose.width, pose.height);
  const ax = ((pose.tiltX ?? 0) * Math.PI) / 180;
  const ay = ((pose.tiltY ?? 0) * Math.PI) / 180;
  const az = ((pose.rotate ?? 0) * Math.PI) / 180;
  const cx = pictureWidth / 2 + (pose.dx ?? 0);
  const cy = pictureHeight / 2 + (pose.dy ?? 0);
  const corners: [number, number][] = [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ];
  return corners.map(([sx, sy]) => {
    let x = (sx * pose.width) / 2;
    let y = (sy * pose.height) / 2;
    let z = 0;
    [y, z] = [y * Math.cos(ax) - z * Math.sin(ax), y * Math.sin(ax) + z * Math.cos(ax)];
    [x, z] = [x * Math.cos(ay) + z * Math.sin(ay), -x * Math.sin(ay) + z * Math.cos(ay)];
    [x, y] = [x * Math.cos(az) - y * Math.sin(az), x * Math.sin(az) + y * Math.cos(az)];
    return { x: cx + (d * x) / (d + z), y: cy + (d * y) / (d + z) };
  }) as Quad;
}

function inside(q: Quad, x: number, y: number): boolean {
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    if ((b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x) < 0) return false;
  }
  return true;
}

export interface Scene {
  width: number;
  height: number;
  quad: Quad;
  /** The cover's colour at (u, v) in 0..1, (0, 0) its top-left corner. */
  cover: (u: number, v: number) => Rgb;
  background: (x: number, y: number) => Rgb;
  /** Standard deviation of the Gaussian noise added to every channel. */
  noise?: number;
  seed?: number;
}

const UNIT: Quad = [
  { x: 0, y: 0 },
  { x: 1, y: 0 },
  { x: 1, y: 1 },
  { x: 0, y: 1 },
];

/** Renders a scene with 3 × 3 supersampling, so the cover's outline is antialiased. */
export function renderScene(scene: Scene): PixelImage {
  const { width, height, quad } = scene;
  const toCover = homography(quad, UNIT);
  const rand = random(scene.seed ?? 1);
  const noise = scene.noise ?? 0;
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      let r = 0;
      let g = 0;
      let b = 0;
      for (let sy = 0; sy < 3; sy++) {
        for (let sx = 0; sx < 3; sx++) {
          const px = x + (sx + 0.5) / 3;
          const py = y + (sy + 0.5) / 3;
          let c: Rgb;
          if (inside(quad, px, py)) {
            const p: Point = applyHomography(toCover, { x: px, y: py });
            c = scene.cover(Math.min(1, Math.max(0, p.x)), Math.min(1, Math.max(0, p.y)));
          } else {
            c = scene.background(px, py);
          }
          r += c[0];
          g += c[1];
          b += c[2];
        }
      }
      const o = (y * width + x) * 4;
      data[o] = r / 9 + noise * gaussian(rand);
      data[o + 1] = g / 9 + noise * gaussian(rand);
      data[o + 2] = b / 9 + noise * gaussian(rand);
      data[o + 3] = 255;
    }
  }
  return { data, width, height };
}

export const solid =
  (c: Rgb) =>
  (): Rgb =>
    c;

/** A cover with a title, a picture block and an author line, all with sharp edges of their own. */
export function bookCover(base: Rgb, ink: Rgb, picture: Rgb): (u: number, v: number) => Rgb {
  return (u, v) => {
    if (v > 0.08 && v < 0.17 && u > 0.12 && u < 0.88 && Math.floor(u * 36) % 3 !== 0) return ink;
    if (u > 0.15 && u < 0.85 && v > 0.26 && v < 0.74) {
      const t = 0.6 + 0.4 * Math.sin(u * 9) * Math.cos(v * 7);
      return [picture[0] * t, picture[1] * t, picture[2] * t];
    }
    if (v > 0.82 && v < 0.88 && u > 0.3 && u < 0.7 && Math.floor(u * 50) % 4 !== 0) return ink;
    return base;
  };
}

/** Wood grain: wavy streaks along x. */
export function wood(base: Rgb): (x: number, y: number) => Rgb {
  return (x, y) => {
    const t = 18 * Math.sin(y * 0.35 + 2.5 * Math.sin(x * 0.03)) + 8 * Math.sin(y * 1.7 + x * 0.05);
    return [base[0] + t, base[1] + t * 0.8, base[2] + t * 0.6];
  };
}

/** Brightness falling from `left` at the left edge to `right` at the right edge. */
export function ramp(left: number, right: number, width: number): (x: number) => Rgb {
  return (x) => {
    const v = left + ((right - left) * x) / width;
    return [v, v, v];
  };
}

/** The largest distance between a detected corner and the true one. */
export function cornerError(found: Quad, truth: Quad): number {
  return Math.max(...found.map((p, i) => Math.hypot(p.x - truth[i].x, p.y - truth[i].y)));
}

/** A box blur of radius `r`, for out-of-focus photos. */
export function blurred(image: PixelImage, r: number): PixelImage {
  const { width: w, height: h, data } = image;
  const out = new Uint8ClampedArray(data.length);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let r0 = 0;
      let g0 = 0;
      let b0 = 0;
      let n = 0;
      for (let dy = -r; dy <= r; dy++) {
        for (let dx = -r; dx <= r; dx++) {
          const j = (Math.min(h - 1, Math.max(0, y + dy)) * w + Math.min(w - 1, Math.max(0, x + dx))) * 4;
          r0 += data[j];
          g0 += data[j + 1];
          b0 += data[j + 2];
          n++;
        }
      }
      const o = (y * w + x) * 4;
      out[o] = r0 / n;
      out[o + 1] = g0 / n;
      out[o + 2] = b0 / n;
      out[o + 3] = 255;
    }
  }
  return { data: out, width: w, height: h };
}

/** The distance from (x, y) to the nearest side of `q`. */
export function distanceToQuad(q: Quad, x: number, y: number): number {
  let best = Infinity;
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    const len2 = (b.x - a.x) ** 2 + (b.y - a.y) ** 2;
    const t = Math.max(0, Math.min(1, ((x - a.x) * (b.x - a.x) + (y - a.y) * (b.y - a.y)) / len2));
    best = Math.min(best, Math.hypot(x - (a.x + t * (b.x - a.x)), y - (a.y + t * (b.y - a.y))));
  }
  return best;
}

/** A background with the cover's soft shadow, cast `offset` px down and right. */
export function shadowed(q: Quad, base: Rgb, offset = 8, softness = 12): (x: number, y: number) => Rgb {
  return (x, y) => {
    const d = inside(q, x - offset, y - offset) ? 0 : distanceToQuad(q, x - offset, y - offset);
    const s = d < softness ? 0.55 + (0.45 * d) / softness : 1;
    return [base[0] * s, base[1] * s, base[2] * s];
  };
}
