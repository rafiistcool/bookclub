/**
 * Finds a book cover's four corners in a photo, like a document-scanner app.
 * Pure TypeScript on a small RGBA copy of the photo (long side about
 * 320–512 px; larger pictures are box-downscaled first).
 *
 * 1. Lines: a small Gaussian blur, Sobel gradients (per pixel the strongest
 *    colour channel, so a cover that differs from the table only in hue has
 *    edges too), thin edges (non-maximum suppression) above a threshold that
 *    follows the picture's noise, then a Hough transform in which each edge
 *    pixel votes only near its own gradient direction. The strongest roughly
 *    horizontal and roughly vertical lines are paired into quadrilaterals;
 *    the plausible ones (convex, 5–98 % of the picture, a cover's aspect
 *    upright or lying, no extreme corner angles) are scored by how much of each quarter of each
 *    side an edge runs along, then by size, then by the outline's contrast,
 *    and the winner's sides are refitted to their edge pixels.
 * 2. Regions, when the lines are weak: the background's colour from the
 *    picture's border, the pixels that differ from it (Otsu), the largest
 *    connected region, its convex hull and the largest quadrilateral in it,
 *    its sides refitted to the region's outline.
 * 3. A cover that runs off the photo (tight framing): when the lines' winner
 *    is surrounded by something other than the background, an outline of
 *    lines and one or two of the picture's own edges (or of four lines with a
 *    corner off the picture) that holds that same material is the cover's
 *    visible part, and the winner was only its print.
 *
 * The better of the two wins, except that a region inside a fairly well
 * outlined quad is that cover's print. The confidence (0..1) is how much of
 * the sides an edge runs along (of the sides the picture shows; for regions,
 * mixed with how well the quad covers the region); below `MIN_CONFIDENCE`
 * the result is `null`, and the caller starts from `insetQuad()` instead.
 * Covers may be upright or lying (landscape, or turned sideways). Fails on
 * purpose-built traps such as a checked tablecloth, whose grid lines outline
 * quads as well as the cover.
 */

import { clampQuad, distance, isConvex, orderQuad, quadArea, quadSides, scaleQuad, type PixelImage, type Point, type Quad } from "./geometry";

export interface Detection {
  quad: Quad;
  confidence: number;
}

/** Below this the detection is not worth showing. */
export const MIN_CONFIDENCE = 0.35;
/** At or above this the region fallback is not tried. */
const GOOD_ENOUGH = 0.75;
/** Lines this well supported win over a region inside them. */
const OUTLINED = 0.5;
/** The smallest share of the picture a cover may fill: a cover photographed from arm's length still counts. */
const MIN_AREA = 0.05;
const MAX_AREA = 0.98;
/**
 * A photographed cover's width / height: upright or lying (a landscape
 * picture book, a cover turned sideways in the frame), the same range either
 * way.
 */
const MIN_PHOTO_ASPECT = 0.4;
const MAX_PHOTO_ASPECT = 1 / MIN_PHOTO_ASPECT;
const MIN_CORNER_ANGLE = 45;
const MAX_CORNER_ANGLE = 135;
/** Larger pictures are box-downscaled to a long side of at most this. */
const WORK_SIDE = 512;

const DEG = Math.PI / 180;
const COS = new Float64Array(180);
const SIN = new Float64Array(180);
for (let t = 0; t < 180; t++) {
  COS[t] = Math.cos(t * DEG);
  SIN[t] = Math.sin(t * DEG);
}

/** The gradient analysis shared by both detectors. */
interface Edges {
  width: number;
  height: number;
  /** Gradient magnitude per pixel. */
  mag: Float32Array;
  /** Gradient direction per pixel in whole degrees, modulo 180. */
  angle: Uint8Array;
  /** 1 for a thinned edge pixel above the threshold. */
  edge: Uint8Array;
  /** The indices of the edge pixels. */
  list: Int32Array;
}

/** A line n·p = c with |n| = 1. */
interface Line {
  nx: number;
  ny: number;
  c: number;
  votes: number;
  /** The Hough angle of the normal, 0..179. */
  theta: number;
}

// ---------------------------------------------------------------------------
// Pixels

function shrink(image: PixelImage): { image: PixelImage; factor: number } {
  const long = Math.max(image.width, image.height);
  if (long <= WORK_SIDE * 1.25) return { image, factor: 1 };
  const k = Math.ceil(long / WORK_SIDE);
  const w = Math.floor(image.width / k);
  const h = Math.floor(image.height / k);
  const data = new Uint8ClampedArray(w * h * 4);
  const src = image.data;
  const n = k * k;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let r = 0;
      let g = 0;
      let b = 0;
      for (let dy = 0; dy < k; dy++) {
        let i = ((y * k + dy) * image.width + x * k) * 4;
        for (let dx = 0; dx < k; dx++, i += 4) {
          r += src[i];
          g += src[i + 1];
          b += src[i + 2];
        }
      }
      const o = (y * w + x) * 4;
      data[o] = r / n;
      data[o + 1] = g / n;
      data[o + 2] = b / n;
      data[o + 3] = 255;
    }
  }
  return { image: { data, width: w, height: h }, factor: k };
}

/** A separable [1 4 6 4 1] / 16 blur (σ ≈ 1), edges clamped. */
function blur(src: Float32Array, w: number, h: number): Float32Array {
  const tmp = new Float32Array(w * h);
  const out = new Float32Array(w * h);
  for (let y = 0; y < h; y++) {
    const row = y * w;
    for (let x = 0; x < w; x++) {
      const a = src[row + Math.max(0, x - 2)];
      const b = src[row + Math.max(0, x - 1)];
      const c = src[row + x];
      const d = src[row + Math.min(w - 1, x + 1)];
      const e = src[row + Math.min(w - 1, x + 2)];
      tmp[row + x] = (a + 4 * b + 6 * c + 4 * d + e) / 16;
    }
  }
  for (let y = 0; y < h; y++) {
    const r0 = Math.max(0, y - 2) * w;
    const r1 = Math.max(0, y - 1) * w;
    const r2 = y * w;
    const r3 = Math.min(h - 1, y + 1) * w;
    const r4 = Math.min(h - 1, y + 2) * w;
    for (let x = 0; x < w; x++) {
      out[r2 + x] = (tmp[r0 + x] + 4 * tmp[r1 + x] + 6 * tmp[r2 + x] + 4 * tmp[r3 + x] + tmp[r4 + x]) / 16;
    }
  }
  return out;
}

/** The threshold that best splits a histogram into two classes, and how well it does (0..1). */
function otsu(hist: Float64Array): { index: number; separation: number } {
  let total = 0;
  let sum = 0;
  for (let i = 0; i < hist.length; i++) {
    total += hist[i];
    sum += i * hist[i];
  }
  if (total === 0) return { index: 0, separation: 0 };
  const mean = sum / total;
  let variance = 0;
  for (let i = 0; i < hist.length; i++) variance += hist[i] * (i - mean) * (i - mean);
  variance /= total;
  let w0 = 0;
  let s0 = 0;
  let best = 0;
  let index = 0;
  for (let i = 0; i < hist.length; i++) {
    w0 += hist[i];
    s0 += i * hist[i];
    const w1 = total - w0;
    if (w0 === 0 || w1 === 0) continue;
    const m0 = s0 / w0;
    const m1 = (sum - s0) / w1;
    const between = (w0 / total) * (w1 / total) * (m0 - m1) * (m0 - m1);
    if (between > best) {
      best = between;
      index = i;
    }
  }
  return { index, separation: variance > 0 ? best / variance : 0 };
}

function analyse(image: PixelImage): Edges {
  const { width: w, height: h, data } = image;
  const n = w * h;
  // Each colour channel on its own, so a cover that differs from the table
  // only in hue still has edges; a pixel takes its strongest channel.
  const channels: Float32Array[] = [];
  for (let c = 0; c < 3; c++) {
    const plane = new Float32Array(n);
    for (let i = 0, j = c; i < n; i++, j += 4) plane[i] = data[j];
    channels.push(blur(plane, w, h));
  }
  const mag = new Float32Array(n);
  const angle = new Uint8Array(n);
  for (let y = 1; y < h - 1; y++) {
    for (let x = 1; x < w - 1; x++) {
      const i = y * w + x;
      let bestX = 0;
      let bestY = 0;
      let best = -1;
      for (const g of channels) {
        const tl = g[i - w - 1];
        const tr = g[i - w + 1];
        const bl = g[i + w - 1];
        const br = g[i + w + 1];
        const gx = tr + 2 * g[i + 1] + br - tl - 2 * g[i - 1] - bl;
        const gy = bl + 2 * g[i + w] + br - tl - 2 * g[i - w] - tr;
        const m = gx * gx + gy * gy;
        if (m > best) {
          best = m;
          bestX = gx;
          bestY = gy;
        }
      }
      mag[i] = Math.sqrt(best);
      let a = Math.round(Math.atan2(bestY, bestX) / DEG);
      if (a < 0) a += 180;
      if (a >= 180) a -= 180;
      angle[i] = a;
    }
  }

  // The noise level: the median magnitude, from a histogram of whole values.
  let maxMag = 0;
  for (let i = 0; i < n; i++) if (mag[i] > maxMag) maxMag = mag[i];
  const bins = 1024;
  const step = Math.max(maxMag, 1) / bins;
  const hist = new Uint32Array(bins + 1);
  for (let i = 0; i < n; i++) hist[Math.min(bins, Math.floor(mag[i] / step))]++;
  let acc = 0;
  let median = 0;
  for (let b = 0; b <= bins; b++) {
    acc += hist[b];
    if (acc >= n / 2) {
      median = (b + 0.5) * step;
      break;
    }
  }

  // Thin the edges: keep a pixel only where its magnitude peaks across the edge.
  const thin = new Float32Array(n);
  for (let y = 1; y < h - 1; y++) {
    for (let x = 1; x < w - 1; x++) {
      const i = y * w + x;
      const m = mag[i];
      if (m === 0) continue;
      const a = angle[i];
      let o: number;
      if (a < 23 || a >= 158) o = 1;
      else if (a < 68) o = w + 1;
      else if (a < 113) o = w;
      else o = w - 1;
      if (m >= mag[i - o] && m > mag[i + o]) thin[i] = m;
    }
  }

  // Above the noise, and at most about 10 % of the picture.
  let threshold = Math.max(8, 2.5 * median);
  const thinHist = new Uint32Array(bins + 1);
  let thinCount = 0;
  for (let i = 0; i < n; i++) {
    if (thin[i] > 0) {
      thinHist[Math.min(bins, Math.floor(thin[i] / step))]++;
      thinCount++;
    }
  }
  const cap = 0.1 * n;
  if (thinCount > cap) {
    let above = 0;
    for (let b = bins; b >= 0; b--) {
      above += thinHist[b];
      if (above >= cap) {
        threshold = Math.max(threshold, b * step);
        break;
      }
    }
  }
  const edge = new Uint8Array(n);
  const indices: number[] = [];
  for (let i = 0; i < n; i++) {
    if (thin[i] >= threshold) {
      edge[i] = 1;
      indices.push(i);
    }
  }
  return { width: w, height: h, mag, angle, edge, list: Int32Array.from(indices) };
}

// ---------------------------------------------------------------------------
// Lines and quads

function angleGap(a: number, b: number): number {
  const d = Math.abs(a - b) % 180;
  return d > 90 ? 180 - d : d;
}

function intersect(a: Line, b: Line): Point | null {
  const det = a.nx * b.ny - a.ny * b.nx;
  if (Math.abs(det) < 1e-6) return null;
  return { x: (a.c * b.ny - a.ny * b.c) / det, y: (a.nx * b.c - a.c * b.nx) / det };
}

function houghLines(edges: Edges, maxLines: number): Line[] {
  const { width: w, height: h, list, angle } = edges;
  const cx = w / 2;
  const cy = h / 2;
  const maxR = Math.ceil(Math.hypot(w, h) / 2) + 2;
  const nR = 2 * maxR + 1;
  const acc = new Float32Array(180 * nR);
  const spread = 4;
  for (let k = 0; k < list.length; k++) {
    const i = list[k];
    const dx = (i % w) + 0.5 - cx;
    const dy = Math.floor(i / w) + 0.5 - cy;
    const a = angle[i];
    for (let d = -spread; d <= spread; d++) {
      let t = a + d;
      if (t < 0) t += 180;
      else if (t >= 180) t -= 180;
      const r = Math.round(dx * COS[t] + dy * SIN[t]) + maxR;
      acc[t * nR + r] += 1;
    }
  }
  // A line's votes spread over neighbouring distances; sum three of them.
  const score = new Float32Array(180 * nR);
  for (let t = 0; t < 180; t++) {
    const base = t * nR;
    for (let r = 1; r < nR - 1; r++) score[base + r] = acc[base + r - 1] + acc[base + r] + acc[base + r + 1];
  }
  const at = (t: number, r: number): number => {
    if (t < 0) {
      t += 180;
      r = nR - 1 - r;
    } else if (t >= 180) {
      t -= 180;
      r = nR - 1 - r;
    }
    if (r < 0 || r >= nR) return 0;
    return score[t * nR + r];
  };
  const minVotes = Math.max(10, 0.12 * Math.min(w, h));
  const peaks: { t: number; r: number; s: number }[] = [];
  for (let t = 0; t < 180; t++) {
    for (let r = 1; r < nR - 1; r++) {
      const s = score[t * nR + r];
      if (s < minVotes) continue;
      let peak = true;
      for (let dt = -2; dt <= 2 && peak; dt++) {
        for (let dr = -3; dr <= 3; dr++) {
          if ((dt !== 0 || dr !== 0) && at(t + dt, r + dr) > s) {
            peak = false;
            break;
          }
        }
      }
      if (peak) peaks.push({ t, r: r - maxR, s });
    }
  }
  peaks.sort((a, b) => b.s - a.s);
  const lines: Line[] = [];
  for (const p of peaks) {
    if (lines.length >= maxLines) break;
    const nx = COS[p.t];
    const ny = SIN[p.t];
    const line: Line = { nx, ny, c: p.r + nx * cx + ny * cy, votes: p.s, theta: p.t };
    // Drop near-duplicates: about the same angle and the same place in the picture.
    const duplicate = lines.some((l) => {
      if (angleGap(l.theta, line.theta) > 6) return false;
      const sign = l.nx * line.nx + l.ny * line.ny < 0 ? -1 : 1;
      const dl = l.nx * cx + l.ny * cy - l.c;
      const dn = sign * (line.nx * cx + line.ny * cy - line.c);
      return Math.abs(dl - dn) < 8;
    });
    if (!duplicate) lines.push(line);
  }
  return lines;
}

/** Corner angle at b in degrees. */
function cornerAngle(a: Point, b: Point, c: Point): number {
  const ux = a.x - b.x;
  const uy = a.y - b.y;
  const vx = c.x - b.x;
  const vy = c.y - b.y;
  const cos = (ux * vx + uy * vy) / (Math.hypot(ux, uy) * Math.hypot(vx, vy));
  return Math.acos(Math.max(-1, Math.min(1, cos))) / DEG;
}

/** Whether a quad could be a photographed cover in a w × h picture, its corners at most `margin` (× the long side) outside it. */
function plausible(q: Quad, w: number, h: number, margin = 0.02): boolean {
  margin *= Math.max(w, h);
  for (const p of q) {
    if (!(p.x >= -margin && p.x <= w + margin && p.y >= -margin && p.y <= h + margin)) return false;
  }
  if (!isConvex(q)) return false;
  const area = quadArea(q) / (w * h);
  if (area < MIN_AREA || area > MAX_AREA) return false;
  const sides = quadSides(q);
  const aspect = sides.width / sides.height;
  if (aspect < MIN_PHOTO_ASPECT || aspect > MAX_PHOTO_ASPECT) return false;
  for (let i = 0; i < 4; i++) {
    const angle = cornerAngle(q[(i + 3) % 4], q[i], q[(i + 1) % 4]);
    if (angle < MIN_CORNER_ANGLE || angle > MAX_CORNER_ANGLE) return false;
  }
  const top = Math.hypot(q[1].x - q[0].x, q[1].y - q[0].y);
  const bottom = Math.hypot(q[2].x - q[3].x, q[2].y - q[3].y);
  const left = Math.hypot(q[3].x - q[0].x, q[3].y - q[0].y);
  const right = Math.hypot(q[2].x - q[1].x, q[2].y - q[1].y);
  return Math.min(top, bottom) / Math.max(top, bottom) > 0.5 && Math.min(left, right) / Math.max(left, right) > 0.5;
}

const SEGMENTS = 4;

/**
 * The share of a side that an edge of the side's direction runs along, for
 * each quarter of the side (its last 4 % at each corner left out). The
 * quarters catch a side that runs on past the cover's corner.
 */
function sideSupport(edges: Edges, a: Point, b: Point, out: Float64Array): void {
  const { width: w, height: h, edge, angle } = edges;
  out.fill(0);
  const len = Math.hypot(b.x - a.x, b.y - a.y);
  if (len < 8) return;
  const ux = (b.x - a.x) / len;
  const uy = (b.y - a.y) / len;
  const nx = -uy;
  const ny = ux;
  let normal = Math.round(Math.atan2(ny, nx) / DEG);
  if (normal < 0) normal += 180;
  if (normal >= 180) normal -= 180;
  const perSegment = Math.max(4, Math.floor(len / (2 * SEGMENTS)));
  const samples = perSegment * SEGMENTS;
  for (let s = 0; s < samples; s++) {
    const t = 0.04 + (0.92 * (s + 0.5)) / samples;
    const px = a.x + (b.x - a.x) * t;
    const py = a.y + (b.y - a.y) * t;
    for (let k = -2; k <= 2; k++) {
      const x = Math.floor(px + k * nx);
      const y = Math.floor(py + k * ny);
      if (x < 0 || y < 0 || x >= w || y >= h) continue;
      const i = y * w + x;
      if (edge[i] && angleGap(angle[i], normal) <= 20) {
        out[Math.floor(s / perSegment)] += 1 / perSegment;
        break;
      }
    }
  }
}

const scratch = [new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS)];

/**
 * How well edges outline the quad, 0..1: the mean support of its sides, mixed
 * with its weakest side and its weakest quarter of a side.
 */
function supportOf(edges: Edges, q: Quad): number {
  for (let i = 0; i < 4; i++) sideSupport(edges, q[i], q[(i + 1) % 4], scratch[i]);
  return combinedSupport(scratch);
}

/** A weighted total-least-squares line through points, or null for too few. */
function fitLine(xs: number[], ys: number[], ws: number[]): Line | null {
  let sw = 0;
  let mx = 0;
  let my = 0;
  for (let i = 0; i < xs.length; i++) {
    sw += ws[i];
    mx += ws[i] * xs[i];
    my += ws[i] * ys[i];
  }
  if (sw <= 0) return null;
  mx /= sw;
  my /= sw;
  let sxx = 0;
  let syy = 0;
  let sxy = 0;
  for (let i = 0; i < xs.length; i++) {
    const dx = xs[i] - mx;
    const dy = ys[i] - my;
    sxx += ws[i] * dx * dx;
    syy += ws[i] * dy * dy;
    sxy += ws[i] * dx * dy;
  }
  // The direction is the principal axis; the normal is perpendicular to it.
  const theta = 0.5 * Math.atan2(2 * sxy, sxx - syy);
  const nx = -Math.sin(theta);
  const ny = Math.cos(theta);
  return { nx, ny, c: nx * mx + ny * my, votes: sw, theta: 0 };
}

/**
 * Refits each side of `q` to the pixels near it that `weight` picks (by index,
 * 0 for none), within `band` pixels and away from the corners, and returns the
 * quad of the refitted lines. A side with too few pixels stays as it was.
 */
function refit(q: Quad, w: number, h: number, band: number, weight: (i: number, normal: number) => number): Quad {
  const lines: Line[] = [];
  for (let s = 0; s < 4; s++) {
    const a = q[s];
    const b = q[(s + 1) % 4];
    const len = Math.hypot(b.x - a.x, b.y - a.y);
    const ux = (b.x - a.x) / len;
    const uy = (b.y - a.y) / len;
    const nx = -uy;
    const ny = ux;
    let normal = Math.round(Math.atan2(ny, nx) / DEG);
    if (normal < 0) normal += 180;
    if (normal >= 180) normal -= 180;
    const xs: number[] = [];
    const ys: number[] = [];
    const ws: number[] = [];
    const seen = new Set<number>();
    const steps = Math.floor(len);
    for (let t = Math.ceil(0.06 * steps); t <= Math.floor(0.94 * steps); t++) {
      for (let k = -band; k <= band; k++) {
        const x = Math.floor(a.x + ux * t + nx * k);
        const y = Math.floor(a.y + uy * t + ny * k);
        if (x < 0 || y < 0 || x >= w || y >= h) continue;
        const i = y * w + x;
        if (seen.has(i)) continue;
        seen.add(i);
        const wt = weight(i, normal);
        if (wt > 0) {
          xs.push(x + 0.5);
          ys.push(y + 0.5);
          ws.push(wt);
        }
      }
    }
    const fitted = xs.length >= 0.3 * len ? fitLine(xs, ys, ws) : null;
    lines.push(fitted ?? { nx, ny, c: nx * a.x + ny * a.y, votes: 0, theta: 0 });
  }
  const corners: Point[] = [];
  for (let s = 0; s < 4; s++) {
    // Corner s joins side s - 1 and side s.
    const p = intersect(lines[(s + 3) % 4], lines[s]);
    if (!p) return q;
    corners.push(p);
  }
  return corners as Quad;
}

function edgeWeight(edges: Edges): (i: number, normal: number) => number {
  return (i, normal) => (edges.edge[i] && angleGap(edges.angle[i], normal) <= 20 ? edges.mag[i] : 0);
}

function refineByEdges(edges: Edges, q: Quad): Quad {
  let best = q;
  let bestSupport = supportOf(edges, q);
  let current = q;
  for (const band of [3, 2]) {
    const next = refit(current, edges.width, edges.height, band, edgeWeight(edges));
    if (!plausible(next, edges.width, edges.height)) break;
    const support = supportOf(edges, next);
    if (support + 1e-9 >= bestSupport) {
      best = next;
      bestSupport = support;
    }
    current = next;
  }
  return best;
}

/**
 * For one line, whether an edge of the line's direction lies within 2 px at
 * each 1 px step along it, as prefix sums: the support of any stretch of the
 * line is then two lookups.
 */
interface Profile {
  /** The foot point of the line and its direction. */
  px: number;
  py: number;
  dx: number;
  dy: number;
  /** The `t` of index 0. */
  start: number;
  /** prefix[j] = hits at indices below j. */
  prefix: Float64Array;
  /** The same for the hits' gradient magnitudes. */
  strength: Float64Array;
}

function profileOf(edges: Edges, l: Line): Profile {
  const { width: w, height: h, edge, angle } = edges;
  const dx = -l.ny;
  const dy = l.nx;
  const px = l.nx * l.c;
  const py = l.ny * l.c;
  const diag = Math.hypot(w, h);
  const tc = (w / 2 - px) * dx + (h / 2 - py) * dy;
  const start = Math.floor(tc - diag / 2) - 1;
  const n = Math.ceil(diag) + 3;
  const prefix = new Float64Array(n + 1);
  const strength = new Float64Array(n + 1);
  let normal = Math.round(Math.atan2(l.ny, l.nx) / DEG);
  if (normal < 0) normal += 180;
  if (normal >= 180) normal -= 180;
  for (let j = 0; j < n; j++) {
    const t = start + j + 0.5;
    const x0 = px + dx * t;
    const y0 = py + dy * t;
    let hit = 0;
    let m = 0;
    if (x0 >= -2 && y0 >= -2 && x0 <= w + 2 && y0 <= h + 2) {
      for (let k = -2; k <= 2; k++) {
        const x = Math.floor(x0 + k * l.nx);
        const y = Math.floor(y0 + k * l.ny);
        if (x < 0 || y < 0 || x >= w || y >= h) continue;
        const i = y * w + x;
        if (edge[i] && angleGap(angle[i], normal) <= 20) {
          hit = 1;
          if (edges.mag[i] > m) m = edges.mag[i];
        }
      }
    }
    prefix[j + 1] = prefix[j] + hit;
    strength[j + 1] = strength[j] + m;
  }
  return { px, py, dx, dy, start, prefix, strength };
}

/**
 * `sideSupport()`'s quarters for the stretch of a profiled line between two
 * of its points; returns the mean gradient magnitude of the stretch's hits.
 */
function profileSupport(p: Profile, a: Point, b: Point, out: Float64Array): number {
  let ta = (a.x - p.px) * p.dx + (a.y - p.py) * p.dy - p.start;
  let tb = (b.x - p.px) * p.dx + (b.y - p.py) * p.dy - p.start;
  if (ta > tb) [ta, tb] = [tb, ta];
  const len = tb - ta;
  const n = p.prefix.length - 1;
  const from = ta + 0.04 * len;
  const step = (0.92 * len) / SEGMENTS;
  for (let k = 0; k < SEGMENTS; k++) {
    const i = Math.min(n, Math.max(0, Math.round(from + k * step)));
    const j = Math.min(n, Math.max(0, Math.round(from + (k + 1) * step)));
    out[k] = j > i ? (p.prefix[j] - p.prefix[i]) / (j - i) : 0;
  }
  const i = Math.min(n, Math.max(0, Math.round(from)));
  const j = Math.min(n, Math.max(0, Math.round(from + SEGMENTS * step)));
  const hits = p.prefix[j] - p.prefix[i];
  return hits > 0 ? (p.strength[j] - p.strength[i]) / hits : 0;
}

function combinedSupport(sides: Float64Array[]): number {
  let sum = 0;
  let minSide = 1;
  let minSegment = 1;
  for (const segs of sides) {
    let side = 0;
    for (let k = 0; k < SEGMENTS; k++) {
      side += segs[k] / SEGMENTS;
      if (segs[k] < minSegment) minSegment = segs[k];
    }
    sum += side;
    if (side < minSide) minSide = side;
  }
  return 0.4 * (sum / sides.length) + 0.3 * minSide + 0.3 * minSegment;
}

const PER_GROUP = 24;
/** An enclosing outline needs at least this support on its own... */
const ENCLOSING_SUPPORT = 0.55;
/** ...and this share of the enclosed one's... */
const ENCLOSING_SHARE = 0.7;
/** ...and this much more area (a shadow's outline around the cover has less). */
const ENCLOSING_GROWTH = 1.3;

function linesDetection(edges: Edges, image?: PixelImage): Detection | null {
  const { width: w, height: h } = edges;
  const lines = houghLines(edges, 64);
  const cx = w / 2;
  const cy = h / 2;
  const horizontal: Ranked[] = [];
  const vertical: Ranked[] = [];
  for (const l of lines) {
    if (l.theta >= 45 && l.theta < 135) {
      if (horizontal.length < PER_GROUP) horizontal.push({ ...l, pos: (l.c - l.nx * cx) / l.ny, profile: profileOf(edges, l) });
    } else if (vertical.length < PER_GROUP) {
      vertical.push({ ...l, pos: (l.c - l.ny * cy) / l.nx, profile: profileOf(edges, l) });
    }
  }
  // Top to bottom and left to right, so a pair (i < j) is always (top, bottom) or (left, right).
  horizontal.sort((a, b) => a.pos - b.pos);
  vertical.sort((a, b) => a.pos - b.pos);
  const nv = vertical.length;
  const crossing: (Point | null)[] = [];
  for (const hl of horizontal) for (const vl of vertical) crossing.push(intersect(hl, vl));
  const sides = [new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS)];
  type Candidate = { quad: Quad; score: number; support: number; area: number };
  let best: Candidate | null = null;
  /** The well-outlined candidates, for the enclosing outline below. */
  const outlined: Candidate[] = [];
  for (let i = 0; i < horizontal.length; i++) {
    for (let j = i + 1; j < horizontal.length; j++) {
      const top = horizontal[i];
      const bottom = horizontal[j];
      if (angleGap(top.theta, bottom.theta) > 40) continue;
      for (let k = 0; k < nv; k++) {
        for (let m = k + 1; m < nv; m++) {
          const left = vertical[k];
          const right = vertical[m];
          if (angleGap(left.theta, right.theta) > 40) continue;
          const tl = crossing[i * nv + k];
          const tr = crossing[i * nv + m];
          const br = crossing[j * nv + m];
          const bl = crossing[j * nv + k];
          if (!tl || !tr || !br || !bl) continue;
          const quad: Quad = [tl, tr, br, bl];
          if (!plausible(quad, w, h)) continue;
          const strength =
            profileSupport(top.profile!, tl, tr, sides[0]) +
            profileSupport(right.profile!, tr, br, sides[1]) +
            profileSupport(bottom.profile!, br, bl, sides[2]) +
            profileSupport(left.profile!, bl, tl, sides[3]);
          const support = combinedSupport(sides);
          if (support < 0.2) continue;
          // Well outlined first, then large; the contrast of the outline
          // breaks near ties (a cover's sharp side against its soft shadow).
          const area = quadArea(quad) / (w * h);
          const score = support * support * Math.sqrt(area) * Math.pow(strength, 0.25);
          const candidate: Candidate = { quad, score, support, area };
          if (support >= ENCLOSING_SUPPORT) outlined.push(candidate);
          if (!best || score > best.score) best = candidate;
        }
      }
    }
  }
  if (!best) return null;
  // A cover's print (a picture block, a band of colour) outlines a rectangle
  // of its own, often more sharply than the cover does against a similar
  // table. An outline that encloses the winner, is clearly larger and is
  // still well supported is the cover around that print: take it, and look
  // again from there.
  for (;;) {
    const inner: Candidate = best;
    let outer: Candidate | null = null;
    for (const c of outlined) {
      if (c.area < ENCLOSING_GROWTH * inner.area || c.support < ENCLOSING_SHARE * inner.support) continue;
      if (!inner.quad.every((p) => insideQuad(c.quad, p.x, p.y))) continue;
      if (!outer || c.score > outer.score) outer = c;
    }
    if (!outer) break;
    best = outer;
  }
  if (image) {
    const cut = runOff(image, edges, horizontal, vertical, best.quad, best.area);
    if (cut) {
      const quad = refineByEdges(edges, cut.quad);
      // The confidence of the sides the picture shows, as far as it shows them.
      const shown: Float64Array[] = [];
      for (let i = 0; i < 4; i++) {
        const part = cut.border[i] ? null : clipToPicture(quad[i], quad[(i + 1) % 4], edges.width, edges.height);
        if (!part) continue;
        sideSupport(edges, part[0], part[1], scratch[i]);
        shown.push(scratch[i]);
      }
      return { quad, confidence: shown.length >= 2 ? combinedSupport(shown) : 0 };
    }
  }
  const quad = refineByEdges(edges, best.quad);
  return { quad, confidence: supportOf(edges, quad) };
}

// ---------------------------------------------------------------------------
// A cover that runs off the photo

/** Two colours this far apart (RGB distance) are different materials. */
const COLOUR_GAP = 40;
/**
 * The visible part of a cover may be only a little larger than the print
 * inside it; the colour checks, not the size, tell them apart.
 */
const RUN_OFF_GROWTH = 1.1;
/** How far (× the picture's long side) the corner of a cover that runs off may lie outside the picture. */
const RUN_OFF_MARGIN = 0.25;
/** A sample this far from a colour does not match it. */
const SAMPLE_GAP = 35;

type Rgb = [number, number, number];

function colourDistance(a: Rgb, b: Rgb): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

/** The per-channel median of colour samples. */
function medianColour(samples: Rgb[]): Rgb {
  const channel = (c: number) => {
    const v = samples.map((s) => s[c]).sort((a, b) => a - b);
    return v.length ? v[v.length >> 1] : 0;
  };
  return [channel(0), channel(1), channel(2)];
}

/** The share of `samples` that do not match `colour`. */
function shareUnlike(samples: Rgb[], colour: Rgb): number {
  if (!samples.length) return 0;
  let far = 0;
  for (const s of samples) if (colourDistance(s, colour) > SAMPLE_GAP) far++;
  return far / samples.length;
}

/**
 * The pixels of a band along the side a → b of a quad in `Quad` order, from
 * `from` to `to` px inwards (negative: outside the quad), leaving out the
 * last 10 % at each corner. Points outside the picture are skipped.
 */
function band(image: PixelImage, a: Point, b: Point, from: number, to: number): Rgb[] {
  const { width: w, height: h, data } = image;
  const len = Math.hypot(b.x - a.x, b.y - a.y);
  const out: Rgb[] = [];
  if (!(len >= 4)) return out;
  const ux = (b.x - a.x) / len;
  const uy = (b.y - a.y) / len;
  // Clockwise on the screen, the inside lies to the right of a → b.
  const nx = -uy;
  const ny = ux;
  for (let t = 0.1 * len; t <= 0.9 * len; t += 2) {
    for (let k = from; k <= to; k++) {
      const x = Math.floor(a.x + ux * t + nx * k);
      const y = Math.floor(a.y + uy * t + ny * k);
      if (x < 0 || y < 0 || x >= w || y >= h) continue;
      const j = (y * w + x) * 4;
      out.push([data[j], data[j + 1], data[j + 2]]);
    }
  }
  return out;
}

/** The background: the median colour of a ring along the picture's border. */
function borderColour(image: PixelImage): Rgb {
  const { width: w, height: h, data } = image;
  const ring = Math.max(2, Math.round(0.03 * Math.min(w, h)));
  const histR = new Uint32Array(256);
  const histG = new Uint32Array(256);
  const histB = new Uint32Array(256);
  let ringCount = 0;
  for (let y = 0; y < h; y++) {
    const inRow = y < ring || y >= h - ring;
    for (let x = 0; x < w; x++) {
      if (!inRow && x >= ring && x < w - ring) {
        x = w - ring - 1;
        continue;
      }
      const j = (y * w + x) * 4;
      histR[data[j]]++;
      histG[data[j + 1]]++;
      histB[data[j + 2]]++;
      ringCount++;
    }
  }
  const median = (hist: Uint32Array) => {
    let acc = 0;
    for (let i = 0; i < 256; i++) {
      acc += hist[i];
      if (acc >= ringCount / 2) return i;
    }
    return 255;
  };
  return [median(histR), median(histG), median(histB)];
}

type Ranked = Line & { pos: number; profile: Profile | null };

/** The part of the segment a → b inside the w × h picture (Liang–Barsky), or null. */
function clipToPicture(a: Point, b: Point, w: number, h: number): [Point, Point] | null {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  let t0 = 0;
  let t1 = 1;
  for (const [p, q] of [
    [-dx, a.x],
    [dx, w - a.x],
    [-dy, a.y],
    [dy, h - a.y],
  ]) {
    if (p === 0) {
      if (q < 0) return null;
      continue;
    }
    const r = q / p;
    if (p < 0) t0 = Math.max(t0, r);
    else t1 = Math.min(t1, r);
    if (t0 > t1) return null;
  }
  return [
    { x: a.x + t0 * dx, y: a.y + t0 * dy },
    { x: a.x + t1 * dx, y: a.y + t1 * dy },
  ];
}

/**
 * When the cover runs off the photo, its outline has a side (or two) the
 * picture does not show, and its print (a picture block, a band of colour)
 * is the best complete outline. `inner` is that kind of winner when, just
 * outside it, there is something other than the background (the picture's
 * border colour), and an outline around it of the lines and the picture's
 * own edges (one or two of them) holds that same material: well supported
 * where it is a line, the material on the inside of those lines and
 * something else beyond them, and the material reaching the picture's edge
 * where the outline runs along it. Returns that outline, and which of its
 * sides lie on the picture's edge, or null.
 */
function runOff(
  image: PixelImage,
  edges: Edges,
  horizontal: Ranked[],
  vertical: Ranked[],
  inner: Quad,
  innerArea: number,
): { quad: Quad; border: boolean[] } | null {
  const { width: w, height: h } = edges;
  const background = borderColour(image);
  // What lies just outside the winner, on the sides where it is not the background.
  const gap: Rgb[] = [];
  for (let i = 0; i < 4; i++) {
    const outside = band(image, inner[i], inner[(i + 1) % 4], -6, -3);
    if (shareUnlike(outside, background) >= 0.6) gap.push(...outside);
  }
  if (!gap.length) return null;
  const material = medianColour(gap);
  if (colourDistance(material, background) < COLOUR_GAP) return null;

  const edge = (nx: number, ny: number, c: number, theta: number, pos: number): Ranked => ({ nx, ny, c, theta, votes: 0, pos, profile: null });
  const hs = [edge(0, 1, 0, 90, 0), ...horizontal, edge(0, 1, h, 90, h)].sort((a, b) => a.pos - b.pos);
  const vs = [edge(1, 0, 0, 0, 0), ...vertical, edge(1, 0, w, 0, w)].sort((a, b) => a.pos - b.pos);
  const sides = [new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS), new Float64Array(SEGMENTS)];
  let best: { quad: Quad; border: boolean[]; score: number } | null = null;
  for (let i = 0; i < hs.length; i++) {
    for (let j = i + 1; j < hs.length; j++) {
      const top = hs[i];
      const bottom = hs[j];
      if (angleGap(top.theta, bottom.theta) > 40) continue;
      for (let k = 0; k < vs.length; k++) {
        for (let m = k + 1; m < vs.length; m++) {
          const left = vs[k];
          const right = vs[m];
          // Clockwise from the top: top, right, bottom, left.
          const lines = [top, right, bottom, left];
          const border = lines.map((l) => !l.profile);
          const open = border.filter(Boolean).length;
          if (open > 2) continue;
          if (angleGap(left.theta, right.theta) > 40) continue;
          const tl = intersect(top, left);
          const tr = intersect(top, right);
          const br = intersect(bottom, right);
          const bl = intersect(bottom, left);
          if (!tl || !tr || !br || !bl) continue;
          const quad: Quad = [tl, tr, br, bl];
          // Four lines of the cover are a run-off only with a corner off the picture.
          if (open === 0 && quad.every((p) => p.x >= -2 && p.y >= -2 && p.x <= w + 2 && p.y <= h + 2)) continue;
          // The winner often shares lines (and so corners) with the outline.
          if (!inner.every((p) => insideQuad(quad, p.x, p.y, 1))) continue;
          const area = quadArea(quad) / (w * h);
          if (area < RUN_OFF_GROWTH * innerArea || !plausible(quad, w, h, RUN_OFF_MARGIN)) continue;
          const supported: Float64Array[] = [];
          let weak = false;
          for (let s = 0; s < 4 && !weak; s++) {
            const profile = lines[s].profile;
            if (!profile) continue;
            // A side measured where the picture shows it, and it must show most of it.
            const a = quad[s];
            const b = quad[(s + 1) % 4];
            const part = clipToPicture(a, b, w, h);
            if (!part || distance(part[0], part[1]) < 0.5 * distance(a, b)) {
              weak = true;
              break;
            }
            profileSupport(profile, part[0], part[1], sides[s]);
            let mean = 0;
            for (let q = 0; q < SEGMENTS; q++) mean += sides[s][q] / SEGMENTS;
            if (mean < ENCLOSING_SUPPORT) weak = true;
            supported.push(sides[s]);
          }
          if (weak) continue;
          const support = combinedSupport(supported);
          if (support < ENCLOSING_SUPPORT) continue;
          const score = support * support * Math.sqrt(area);
          if (best && score <= best.score) continue;
          // The colours: the same material inside the lines and up to the
          // picture's edge, something else beyond the lines.
          const within: Rgb[] = [];
          const beyond: Rgb[] = [];
          let reaches = true;
          for (let s = 0; s < 4 && reaches; s++) {
            const a = quad[s];
            const b = quad[(s + 1) % 4];
            if (border[s]) {
              reaches = shareUnlike(band(image, a, b, 1, 4), background) >= 0.35;
            } else {
              within.push(...band(image, a, b, 2, 5));
              beyond.push(...band(image, a, b, -5, -2));
            }
          }
          if (!reaches) continue;
          const cover = medianColour(within);
          if (colourDistance(cover, material) > COLOUR_GAP || colourDistance(cover, background) < COLOUR_GAP) continue;
          if (shareUnlike(beyond, cover) < 0.6) continue;
          best = { quad, border, score };
        }
      }
    }
  }
  return best && { quad: best.quad, border: best.border };
}

// ---------------------------------------------------------------------------
// Regions

function convexHull(points: Point[]): Point[] {
  const pts = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  if (pts.length < 3) return pts;
  const cross = (o: Point, a: Point, b: Point) => (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lower: Point[] = [];
  for (const p of pts) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) lower.pop();
    lower.push(p);
  }
  const upper: Point[] = [];
  for (let i = pts.length - 1; i >= 0; i--) {
    const p = pts[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) upper.pop();
    upper.push(p);
  }
  upper.pop();
  lower.pop();
  return lower.concat(upper);
}

/** Drops the hull vertex that adds the least area until `max` are left (Visvalingam). */
function simplify(hull: Point[], max: number): Point[] {
  const pts = [...hull];
  const area = (a: Point, b: Point, c: Point) => Math.abs((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x));
  while (pts.length > max) {
    let worst = 0;
    let least = Infinity;
    for (let i = 0; i < pts.length; i++) {
      const a = area(pts[(i + pts.length - 1) % pts.length], pts[i], pts[(i + 1) % pts.length]);
      if (a < least) {
        least = a;
        worst = i;
      }
    }
    pts.splice(worst, 1);
  }
  return pts;
}

/** The largest quadrilateral with its corners on the (convex, ordered) polygon's vertices. */
function largestQuad(poly: Point[]): Point[] | null {
  const n = poly.length;
  if (n < 4) return null;
  const tri = (a: Point, b: Point, c: Point) => Math.abs((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) / 2;
  let best = -1;
  let pick: Point[] | null = null;
  for (let a = 0; a < n; a++) {
    for (let c = a + 2; c < n; c++) {
      // The diagonal a–c; the best b and d lie farthest from it on each side.
      let bestB = -1;
      let areaB = -1;
      for (let b = a + 1; b < c; b++) {
        const t = tri(poly[a], poly[b], poly[c]);
        if (t > areaB) {
          areaB = t;
          bestB = b;
        }
      }
      let bestD = -1;
      let areaD = -1;
      for (let d = c + 1; d < n + a; d++) {
        const t = tri(poly[a], poly[d % n], poly[c]);
        if (t > areaD) {
          areaD = t;
          bestD = d % n;
        }
      }
      if (bestB < 0 || bestD < 0) continue;
      if (areaB + areaD > best) {
        best = areaB + areaD;
        pick = [poly[a], poly[bestB], poly[c], poly[bestD]];
      }
    }
  }
  return pick;
}

/** Whether (x, y) lies inside the quad, or at most `slack` px outside a side. */
function insideQuad(q: Quad, x: number, y: number, slack = 0): boolean {
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    const cross = (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
    if (cross < (slack ? -slack * Math.hypot(b.x - a.x, b.y - a.y) : 0)) return false;
  }
  return true;
}

/**
 * A 3 × 3 erosion (`keep` 0: a pixel stays only when all its neighbours are
 * set) or dilation (`keep` 1: a pixel is set when any neighbour is).
 * Outside the picture counts as unset.
 */
function morph(src: Uint8Array, w: number, h: number, keep: 0 | 1): Uint8Array {
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      // Erosion looks for an unset neighbour, dilation for a set one.
      let found = false;
      for (let dy = -1; dy <= 1 && !found; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
          const xx = x + dx;
          const yy = y + dy;
          const m = xx >= 0 && yy >= 0 && xx < w && yy < h ? src[yy * w + xx] : 0;
          if (m === keep) {
            found = true;
            break;
          }
        }
      }
      out[y * w + x] = keep === 1 ? (found ? 1 : 0) : found ? 0 : 1;
    }
  }
  return out;
}

function regionDetection(image: PixelImage, edges: Edges): Detection | null {
  const { width: w, height: h, data } = image;
  const n = w * h;
  const [mr, mg, mb] = borderColour(image);
  const raw = new Float32Array(n);
  for (let i = 0, j = 0; i < n; i++, j += 4) {
    raw[i] = Math.hypot(data[j] - mr, data[j + 1] - mg, data[j + 2] - mb);
  }
  const dist = blur(raw, w, h);
  const hist = new Float64Array(256);
  for (let i = 0; i < n; i++) hist[Math.min(255, Math.round(dist[i] / 1.7))]++;
  const split = otsu(hist);
  const threshold = (split.index + 0.5) * 1.7;
  if (threshold < 15 || split.separation < 0.5) return null;
  const selected = new Uint8Array(n);
  for (let i = 0; i < n; i++) selected[i] = dist[i] > threshold ? 1 : 0;
  // Open (erode, then dilate) to drop specks and thin bridges to the background.
  const mask = morph(morph(selected, w, h, 0), w, h, 1);

  // The largest 4-connected region.
  const label = new Int32Array(n);
  const stack = new Int32Array(n);
  let bestLabel = 0;
  let bestSize = 0;
  let next = 0;
  for (let start = 0; start < n; start++) {
    if (!mask[start] || label[start]) continue;
    next++;
    let size = 0;
    let top = 0;
    stack[top++] = start;
    label[start] = next;
    while (top > 0) {
      const i = stack[--top];
      size++;
      const x = i % w;
      if (x > 0 && mask[i - 1] && !label[i - 1]) {
        label[i - 1] = next;
        stack[top++] = i - 1;
      }
      if (x < w - 1 && mask[i + 1] && !label[i + 1]) {
        label[i + 1] = next;
        stack[top++] = i + 1;
      }
      if (i >= w && mask[i - w] && !label[i - w]) {
        label[i - w] = next;
        stack[top++] = i - w;
      }
      if (i + w < n && mask[i + w] && !label[i + w]) {
        label[i + w] = next;
        stack[top++] = i + w;
      }
    }
    if (size > bestSize) {
      bestSize = size;
      bestLabel = next;
    }
  }
  if (bestSize < 0.1 * n) return null;

  // The region's outline (its pixels with a neighbour outside) and the corners of its row ends.
  const outline = new Uint8Array(n);
  const ends: Point[] = [];
  for (let y = 0; y < h; y++) {
    let first = -1;
    let last = -1;
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      if (label[i] !== bestLabel) continue;
      if (first < 0) first = x;
      last = x;
      if (
        x === 0 ||
        y === 0 ||
        x === w - 1 ||
        y === h - 1 ||
        label[i - 1] !== bestLabel ||
        label[i + 1] !== bestLabel ||
        label[i - w] !== bestLabel ||
        label[i + w] !== bestLabel
      ) {
        outline[i] = 1;
      }
    }
    if (first >= 0) {
      ends.push({ x: first, y }, { x: first, y: y + 1 }, { x: last + 1, y }, { x: last + 1, y: y + 1 });
    }
  }
  const corners = largestQuad(simplify(convexHull(ends), 24));
  if (!corners) return null;
  let quad = orderQuad(corners);
  if (!isConvex(quad)) return null;
  for (const band of [4, 2]) {
    const next = refit(quad, w, h, band, (i) => outline[i]);
    if (isConvex(next)) quad = next;
  }
  if (!plausible(quad, w, h)) return null;

  // How well the quad covers the region: intersection over union.
  let inter = 0;
  let inQuad = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const inside = insideQuad(quad, x + 0.5, y + 0.5);
      const inRegion = label[y * w + x] === bestLabel;
      if (inside) inQuad++;
      if (inside && inRegion) inter++;
    }
  }
  const iou = inter / (inQuad + bestSize - inter);
  const confidence = 0.5 * supportOf(edges, quad) + 0.5 * iou * Math.min(1, split.separation / 0.8);
  return { quad, confidence };
}

// ---------------------------------------------------------------------------

/** The line detector alone (exported for tests). */
export function detectByLines(image: PixelImage): Detection | null {
  return linesDetection(analyse(image), image);
}

/** The region detector alone (exported for tests). */
export function detectByRegion(image: PixelImage): Detection | null {
  return regionDetection(image, analyse(image));
}

/**
 * The cover's corners in `image`'s pixel coordinates (top-left, top-right,
 * bottom-right, bottom-left, all inside the picture) and a confidence of
 * 0..1, or null when nothing in the picture looks like a cover.
 */
export function detectCover(image: PixelImage): Detection | null {
  if (image.width < 16 || image.height < 16) return null;
  const { image: work, factor } = shrink(image);
  const edges = analyse(work);
  let best = linesDetection(edges, work);
  if (!best || best.confidence < GOOD_ENOUGH) {
    const region = regionDetection(work, edges);
    // A region inside a fairly well outlined quad is the print on that cover
    // (a cover close to the table's colour, a corner off the picture).
    const print = !!region && !!best && best.confidence >= OUTLINED && region.quad.every((p) => insideQuad(best!.quad, p.x, p.y, 1));
    if (region && !print && (!best || region.confidence > best.confidence)) best = region;
  }
  if (!best || !(best.confidence >= MIN_CONFIDENCE)) return null;
  const quad = clampQuad(orderQuad(factor === 1 ? best.quad : scaleQuad(best.quad, factor)), image.width, image.height);
  return { quad, confidence: Math.min(1, Math.max(0, best.confidence)) };
}
