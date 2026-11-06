/**
 * The perspective warp of the cover scan: a projective transform (homography)
 * that maps the photographed quadrilateral onto an upright rectangle, and the
 * resampling that draws it. Pure TypeScript on RGBA arrays, no canvas.
 */

import type { PixelImage, Point, Quad } from "./geometry";

type Matrix3 = number[];

function multiply(a: Matrix3, b: Matrix3): Matrix3 {
  const out = new Array<number>(9);
  for (let r = 0; r < 3; r++) {
    for (let c = 0; c < 3; c++) {
      out[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
    }
  }
  return out;
}

/**
 * Hartley's normalisation: moves the points' centroid to the origin and
 * scales their mean distance from it to √2, which keeps the linear system
 * well conditioned for pixel coordinates in the thousands.
 */
function normaliser(q: Quad): { forward: Matrix3; inverse: Matrix3 } {
  const cx = (q[0].x + q[1].x + q[2].x + q[3].x) / 4;
  const cy = (q[0].y + q[1].y + q[2].y + q[3].y) / 4;
  let mean = 0;
  for (const p of q) mean += Math.hypot(p.x - cx, p.y - cy) / 4;
  const s = mean > 0 ? Math.SQRT2 / mean : 1;
  return {
    forward: [s, 0, -s * cx, 0, s, -s * cy, 0, 0, 1],
    inverse: [1 / s, 0, cx, 0, 1 / s, cy, 0, 0, 1],
  };
}

function transform(m: Matrix3, p: Point): Point {
  const w = m[6] * p.x + m[7] * p.y + m[8];
  return { x: (m[0] * p.x + m[1] * p.y + m[2]) / w, y: (m[3] * p.x + m[4] * p.y + m[5]) / w };
}

/** Solves the 8 × 8 system by Gaussian elimination with partial pivoting. */
function solve(a: number[][], b: number[]): number[] {
  const n = b.length;
  const m = a.map((row, i) => [...row, b[i]]);
  for (let col = 0; col < n; col++) {
    let pivot = col;
    for (let r = col + 1; r < n; r++) {
      if (Math.abs(m[r][col]) > Math.abs(m[pivot][col])) pivot = r;
    }
    if (!(Math.abs(m[pivot][col]) > 1e-10)) {
      throw new RangeError("the corners are degenerate: three of them lie on one line");
    }
    [m[col], m[pivot]] = [m[pivot], m[col]];
    for (let r = col + 1; r < n; r++) {
      const f = m[r][col] / m[col][col];
      if (f === 0) continue;
      for (let c = col; c <= n; c++) m[r][c] -= f * m[col][c];
    }
  }
  const x = new Array<number>(n).fill(0);
  for (let r = n - 1; r >= 0; r--) {
    let sum = m[r][n];
    for (let c = r + 1; c < n; c++) sum -= m[r][c] * x[c];
    x[r] = sum / m[r][r];
  }
  return x;
}

/**
 * The homography that maps each corner of `from` onto the same corner of
 * `to`, as a row-major 3 × 3 matrix scaled so that its last entry is 1
 * (when it can be). Throws a `RangeError` when three corners of either quad
 * lie on one line.
 */
export function homography(from: Quad, to: Quad): number[] {
  const nf = normaliser(from);
  const nt = normaliser(to);
  const a: number[][] = [];
  const b: number[] = [];
  for (let i = 0; i < 4; i++) {
    const p = transform(nf.forward, from[i]);
    const q = transform(nt.forward, to[i]);
    a.push([p.x, p.y, 1, 0, 0, 0, -q.x * p.x, -q.x * p.y]);
    b.push(q.x);
    a.push([0, 0, 0, p.x, p.y, 1, -q.y * p.x, -q.y * p.y]);
    b.push(q.y);
  }
  const hn = [...solve(a, b), 1];
  // Three corners on one line can still solve the system, with a singular matrix.
  const det =
    hn[0] * (hn[4] * hn[8] - hn[5] * hn[7]) - hn[1] * (hn[3] * hn[8] - hn[5] * hn[6]) + hn[2] * (hn[3] * hn[7] - hn[4] * hn[6]);
  if (!(Math.abs(det) > 1e-9)) {
    throw new RangeError("the corners are degenerate: three of them lie on one line");
  }
  const h = multiply(nt.inverse, multiply(hn, nf.forward));
  const scale = Math.abs(h[8]) > 1e-12 ? h[8] : Math.hypot(...h);
  const out = h.map((v) => v / scale);
  if (out.some((v) => !Number.isFinite(v))) {
    throw new RangeError("the corners are degenerate: three of them lie on one line");
  }
  return out;
}

/** Applies a homography from `homography()` to a point. */
export function applyHomography(h: number[], p: Point): Point {
  return transform(h, p);
}

function createPixels(width: number, height: number): ImageData {
  const data = new Uint8ClampedArray(width * height * 4);
  if (typeof ImageData === "function") return new ImageData(data, width, height);
  // jsdom has no ImageData; the shape is all the callers read.
  return { data, width, height, colorSpace: "srgb" } as ImageData;
}

/**
 * Draws the part of `source` inside `quad` (source pixel coordinates, in the
 * order of `Quad`) onto a new `width` × `height` image, the quad's corners on
 * the image's corners. Bilinear sampling; points outside the source take the
 * nearest edge pixel. The output is opaque.
 */
export function warpQuad(source: PixelImage, quad: Quad, width: number, height: number): ImageData {
  const outW = Math.max(1, Math.round(width));
  const outH = Math.max(1, Math.round(height));
  const out = createPixels(outW, outH);
  const dst = out.data;
  const src = source.data;
  const sw = source.width;
  const sh = source.height;
  if (sw < 1 || sh < 1) return out;
  const h = homography(
    [
      { x: 0, y: 0 },
      { x: outW, y: 0 },
      { x: outW, y: outH },
      { x: 0, y: outH },
    ],
    quad,
  );
  const maxX = sw - 1;
  const maxY = sh - 1;
  let o = 0;
  for (let y = 0; y < outH; y++) {
    const yc = y + 0.5;
    // Along a row the three homogeneous terms grow linearly with x.
    let nx = h[0] * 0.5 + h[1] * yc + h[2];
    let ny = h[3] * 0.5 + h[4] * yc + h[5];
    let nw = h[6] * 0.5 + h[7] * yc + h[8];
    for (let x = 0; x < outW; x++) {
      // Source pixel centres sit at i + 0.5.
      let u = nx / nw - 0.5;
      let v = ny / nw - 0.5;
      if (!(u > 0)) u = 0;
      else if (u > maxX) u = maxX;
      if (!(v > 0)) v = 0;
      else if (v > maxY) v = maxY;
      const x0 = Math.floor(u);
      const y0 = Math.floor(v);
      const x1 = x0 < maxX ? x0 + 1 : x0;
      const y1 = y0 < maxY ? y0 + 1 : y0;
      const fx = u - x0;
      const fy = v - y0;
      const i00 = (y0 * sw + x0) * 4;
      const i10 = (y0 * sw + x1) * 4;
      const i01 = (y1 * sw + x0) * 4;
      const i11 = (y1 * sw + x1) * 4;
      const w00 = (1 - fx) * (1 - fy);
      const w10 = fx * (1 - fy);
      const w01 = (1 - fx) * fy;
      const w11 = fx * fy;
      dst[o] = src[i00] * w00 + src[i10] * w10 + src[i01] * w01 + src[i11] * w11;
      dst[o + 1] = src[i00 + 1] * w00 + src[i10 + 1] * w10 + src[i01 + 1] * w01 + src[i11 + 1] * w11;
      dst[o + 2] = src[i00 + 2] * w00 + src[i10 + 2] * w10 + src[i01 + 2] * w01 + src[i11 + 2] * w11;
      dst[o + 3] = 255;
      o += 4;
      nx += h[0];
      ny += h[3];
      nw += h[6];
    }
  }
  return out;
}
