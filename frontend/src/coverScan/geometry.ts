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

/** The narrowest and widest covers the scan produces (width / height). */
export const MIN_ASPECT = 0.4;
export const MAX_ASPECT = 1.0;

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
 * The size of the flattened cover: the quad's mean width and height, its
 * aspect (width / height) clamped to `MIN_ASPECT`..`MAX_ASPECT`, the height
 * (then the long side) taken from the quad's longer mean side and capped at
 * `maxSide`. Never upscales beyond the quad and never returns less than 1 px.
 */
export function outputSize(q: Quad, maxSide = 1600): { width: number; height: number } {
  const sides = quadSides(q);
  const raw = sides.height > 0 ? sides.width / sides.height : MAX_ASPECT;
  const aspect = Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, Number.isFinite(raw) ? raw : MAX_ASPECT));
  const long = Math.min(maxSide, Math.max(sides.width, sides.height));
  const height = Math.max(1, Math.round(long));
  const width = Math.max(1, Math.round(long * aspect));
  return { width, height };
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
