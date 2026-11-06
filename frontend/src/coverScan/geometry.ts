/**
 * Plain geometry for the cover scan: a photographed cover is a quadrilateral
 * in the photo, and the scan maps it onto an upright rectangle (`warp.ts`).
 * Coordinates are source pixels with the origin at the top-left corner of the
 * picture and y growing downwards; pixel (i, j) covers [i, i + 1] × [j, j + 1].
 */

export type Point = { x: number; y: number };

/** Top-left, top-right, bottom-right, bottom-left: clockwise on the screen. */
export type Quad = [Point, Point, Point, Point];

/**
 * RGBA pixels, four bytes each, row by row. A browser `ImageData` is one;
 * tests (jsdom has no `ImageData`) build plain objects of this shape.
 */
export interface PixelImage {
  readonly data: Uint8ClampedArray;
  readonly width: number;
  readonly height: number;
}

/**
 * The narrowest and widest covers the scan produces (width / height): the
 * same range upright and lying, so a landscape picture book keeps its shape.
 */
export const MIN_ASPECT = 0.4;
export const MAX_ASPECT = 1 / MIN_ASPECT;

export function distance(a: Point, b: Point): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

/**
 * Puts four points in the order top-left, top-right, bottom-right,
 * bottom-left: sorted clockwise around their centroid, starting at the one
 * with the smallest x + y. That is unambiguous for covers turned by up to
 * about 40°.
 */
export function orderQuad(points: Point[]): Quad {
  if (points.length !== 4) throw new RangeError(`a quadrilateral needs 4 points, got ${points.length}`);
  const cx = (points[0].x + points[1].x + points[2].x + points[3].x) / 4;
  const cy = (points[0].y + points[1].y + points[2].y + points[3].y) / 4;
  // With y pointing down, a rising atan2 angle runs clockwise on the screen.
  const sorted = points
    .map((p) => ({ x: p.x, y: p.y }))
    .sort((a, b) => Math.atan2(a.y - cy, a.x - cx) - Math.atan2(b.y - cy, b.x - cx));
  let start = 0;
  for (let i = 1; i < 4; i++) {
    if (sorted[i].x + sorted[i].y < sorted[start].x + sorted[start].y) start = i;
  }
  return [sorted[start], sorted[(start + 1) % 4], sorted[(start + 2) % 4], sorted[(start + 3) % 4]];
}

/** The default corners when nothing is detected: the picture, `inset` of each side in. */
export function insetQuad(width: number, height: number, inset = 0.08): Quad {
  const dx = width * inset;
  const dy = height * inset;
  return [
    { x: dx, y: dy },
    { x: width - dx, y: dy },
    { x: width - dx, y: height - dy },
    { x: dx, y: height - dy },
  ];
}

/**
 * True when the quad is strictly convex and in the order of `Quad` (clockwise
 * on the screen). A crossed ("bow-tie") quad, a dent, three corners on one
 * line or a mirrored order (a corner dragged past its neighbour) are false:
 * none of them can be flattened into the cover the reader sees.
 */
export function isConvex(q: Quad): boolean {
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    const c = q[(i + 2) % 4];
    const cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
    // `!(… > 0)` is also true for NaN.
    if (!(cross > 0)) return false;
  }
  return true;
}

/** The area of a simple quad (shoelace formula), positive for the order of `Quad`. */
export function quadArea(q: Quad): number {
  let sum = 0;
  for (let i = 0; i < 4; i++) {
    const a = q[i];
    const b = q[(i + 1) % 4];
    sum += a.x * b.y - b.x * a.y;
  }
  return sum / 2;
}

/** The cover's width and height in the photo: the means of the opposite sides. */
export function quadSides(q: Quad): { width: number; height: number } {
  return {
    width: (distance(q[0], q[1]) + distance(q[3], q[2])) / 2,
    height: (distance(q[0], q[3]) + distance(q[1], q[2])) / 2,
  };
}

/**
 * The size of the flattened cover: `aspect` (width / height; by default the
 * quad's mean width over its mean height, see `coverAspect()` for the
 * cover's own) clamped to `MIN_ASPECT`..`MAX_ASPECT`, the long side taken
 * from the quad's longer mean side and capped at `maxSide`. A portrait aspect
 * puts the long side on the height, a landscape one on the width. Never
 * upscales beyond the quad and never returns less than 1 px.
 */
export function outputSize(q: Quad, maxSide = 1600, aspect?: number): { width: number; height: number } {
  const sides = quadSides(q);
  const raw = aspect !== undefined && Number.isFinite(aspect) && aspect > 0 ? aspect : sides.height > 0 ? sides.width / sides.height : 1;
  const clamped = Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, Number.isFinite(raw) ? raw : 1));
  const long = Math.min(maxSide, Math.max(sides.width, sides.height));
  if (clamped >= 1) return { width: Math.max(1, Math.round(long)), height: Math.max(1, Math.round(long / clamped)) };
  return { width: Math.max(1, Math.round(long * clamped)), height: Math.max(1, Math.round(long)) };
}

/**
 * The quad with its corners moved on by `turns` quarter turns clockwise: the
 * cover `flatten()` makes of it is the one of `q` turned by 90° per turn
 * (1: its left side becomes the top, 2: upside down, 3 or -1: its right side
 * becomes the top). The corners stay on the same points, so the result is
 * as convex as `q`.
 */
export function rotateQuad(q: Quad, turns: number): Quad {
  const t = ((Math.round(turns) % 4) + 4) % 4;
  return [q[(4 - t) % 4], q[(5 - t) % 4], q[(6 - t) % 4], q[(7 - t) % 4]].map((p) => ({ x: p.x, y: p.y })) as Quad;
}

/** The focal length assumed when the quad cannot tell it: a phone's main camera, as a share of the photo's long side. */
export const ASSUMED_FOCAL = 0.8;
/** Focal lengths (× the photo's long side) a measured one may have: an ultra-wide angle to a 5× tele, or a cropped photo. */
const FOCAL_RANGE = [0.25, 4] as const;
/** How far `coverAspect()` moves each corner to measure how much the result depends on it (× the quad's diagonal). */
const JITTER = 0.01;
/** The spread of the jittered estimates (× the estimate) below which it is trusted fully, and above which not at all. */
const TRUSTED_SPREAD = 0.12;
const UNTRUSTED_SPREAD = 0.45;
/**
 * The most the cover's aspect may differ from the photographed one, as a
 * factor either way: a tilt of about 50°. Nobody photographs a cover more
 * askew, and the cap bounds the harm of a wrongly assumed focal length.
 */
const MAX_CORRECTION = 1.6;

type Vec3 = [number, number, number];

function cross(a: Vec3, b: Vec3): Vec3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

/**
 * Zhang and He's rectification ("Whiteboard scanning and image enhancement",
 * 2007): the images of the cover's horizontal and vertical directions, with
 * the principal point (u0, v0) moved to the origin, and the focal length
 * they imply (squared; NaN, infinite or ≤ 0 when the quad cannot tell it,
 * e.g. when two of its sides are parallel).
 */
function vanishing(q: Quad, u0: number, v0: number): { n2: Vec3; n3: Vec3; f2: number } {
  const m1: Vec3 = [q[0].x, q[0].y, 1];
  const m2: Vec3 = [q[1].x, q[1].y, 1];
  const m3: Vec3 = [q[3].x, q[3].y, 1];
  const m4: Vec3 = [q[2].x, q[2].y, 1];
  const k2 = dot(cross(m1, m4), m3) / dot(cross(m2, m4), m3);
  const k3 = dot(cross(m1, m4), m2) / dot(cross(m3, m4), m2);
  const shift = (n: Vec3): Vec3 => [n[0] - u0 * n[2], n[1] - v0 * n[2], n[2]];
  const n2 = shift([k2 * m2[0] - m1[0], k2 * m2[1] - m1[1], k2 - 1]);
  const n3 = shift([k3 * m3[0] - m1[0], k3 * m3[1] - m1[1], k3 - 1]);
  const f2 = -(n2[0] * n3[0] + n2[1] * n3[1]) / (n2[2] * n3[2]);
  return { n2, n3, f2 };
}

function rectifiedAspect(n2: Vec3, n3: Vec3, f2: number): number {
  return Math.sqrt((n2[0] ** 2 + n2[1] ** 2) / f2 + n2[2] ** 2) / Math.sqrt((n3[0] ** 2 + n3[1] ** 2) / f2 + n3[2] ** 2);
}

/** The quad with corner `i` moved by (dx, dy). */
function nudged(q: Quad, i: number, dx: number, dy: number): Quad {
  return q.map((p, j) => (j === i ? { x: p.x + dx, y: p.y + dy } : p)) as Quad;
}

/**
 * The cover's own aspect (width / height) from its corners in a `width` ×
 * `height` photo, undoing the perspective: a cover tilted away from the
 * camera looks shorter than it is. The principal point is taken at the
 * photo's centre; the focal length is measured from the quad when two
 * vanishing points tell it consistently, else assumed (`ASSUMED_FOCAL`).
 * Corners are seldom exact, so the estimate is repeated with each corner
 * moved by 1 % of the diagonal: the more the results scatter, the more the
 * answer leans towards the photographed ratio. It never differs from that
 * ratio by more than a factor of 1.6, and is the photographed ratio itself
 * when nothing can be worked out.
 */
export function coverAspect(q: Quad, width: number, height: number): number {
  const sides = quadSides(q);
  const photographed = sides.height > 0 ? sides.width / sides.height : NaN;
  if (!Number.isFinite(photographed) || photographed <= 0) return 1;
  if (!isConvex(q) || !(width > 0) || !(height > 0)) return photographed;
  const u0 = width / 2;
  const v0 = height / 2;
  const long = Math.max(width, height);
  const assumed = (ASSUMED_FOCAL * long) ** 2;
  const measured = (f2: number) => Number.isFinite(f2) && f2 >= (FOCAL_RANGE[0] * long) ** 2 && f2 <= (FOCAL_RANGE[1] * long) ** 2;

  const base = vanishing(q, u0, v0);
  const step = JITTER * Math.hypot(sides.width, sides.height);
  const jittered: ReturnType<typeof vanishing>[] = [];
  for (let i = 0; i < 4; i++) {
    for (const [dx, dy] of [
      [step, 0],
      [-step, 0],
      [0, step],
      [0, -step],
    ]) {
      jittered.push(vanishing(nudged(q, i, dx, dy), u0, v0));
    }
  }
  // Measure the focal length only when every nudged quad agrees on it within
  // a factor of 1.5; a nearly parallel pair of sides makes it jump around.
  let f2 = assumed;
  if (measured(base.f2) && jittered.every((v) => measured(v.f2))) {
    const fs = jittered.map((v) => v.f2);
    if (Math.sqrt(Math.max(...fs) / Math.min(...fs)) <= 1.5) f2 = base.f2;
  }

  const estimate = rectifiedAspect(base.n2, base.n3, f2);
  if (!Number.isFinite(estimate) || estimate <= 0) return photographed;
  const others = jittered.map((v) => rectifiedAspect(v.n2, v.n3, f2)).filter((a) => Number.isFinite(a) && a > 0);
  let aspect = estimate;
  if (others.length < jittered.length) {
    aspect = photographed;
  } else {
    const spread = (Math.max(...others) - Math.min(...others)) / estimate;
    const lean = Math.min(1, Math.max(0, (spread - TRUSTED_SPREAD) / (UNTRUSTED_SPREAD - TRUSTED_SPREAD)));
    // A geometric blend: the same lean makes 2:3 and 3:2 the same distance away.
    aspect = Math.exp((1 - lean) * Math.log(estimate) + lean * Math.log(photographed));
  }
  return Math.min(photographed * MAX_CORRECTION, Math.max(photographed / MAX_CORRECTION, aspect));
}

/** Multiplies every corner by `factor` (from a downscaled copy back to the photo). */
export function scaleQuad(q: Quad, factor: number): Quad {
  return q.map((p) => ({ x: p.x * factor, y: p.y * factor })) as Quad;
}

/** Keeps every corner inside the picture. */
export function clampQuad(q: Quad, width: number, height: number): Quad {
  return q.map((p) => ({
    x: Math.min(width, Math.max(0, p.x)),
    y: Math.min(height, Math.max(0, p.y)),
  })) as Quad;
}
