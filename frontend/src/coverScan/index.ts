/**
 * The cover scan in the browser: decode the photo upright, read a small copy
 * for `detectCover()`, and once the reader has set the corners, cut the cover
 * out and flatten it into an upright JPEG. Everything runs locally on canvas;
 * nothing is fetched.
 */

import { isConvex, outputSize, quadSides, type Quad } from "./geometry";
import { warpQuad } from "./warp";

export { detectCover, MIN_CONFIDENCE, type Detection } from "./detect";
export { insetQuad, isConvex, orderQuad, outputSize, type PixelImage, type Point, type Quad } from "./geometry";
export { homography, warpQuad } from "./warp";

export type ScanImage = ImageBitmap | HTMLImageElement;

/** The long side of the flattened cover, in pixels. */
export const MAX_OUTPUT_SIDE = 1600;
/** JPEG quality of the flattened cover. */
export const JPEG_QUALITY = 0.9;
/** The most pixels a working canvas gets (iOS Safari refuses canvases above about 16.7 MP). */
const MAX_CANVAS_PIXELS = 12_000_000;

function canvas(width: number, height: number): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D } {
  const el = document.createElement("canvas");
  el.width = width;
  el.height = height;
  const ctx = el.getContext("2d", { willReadFrequently: true });
  if (!ctx) throw new Error("cover scan: this browser has no 2D canvas");
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  return { canvas: el, ctx };
}

function loadImage(file: File): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(file);
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("cover scan: the photo could not be read"));
    img.src = url;
  }).finally(() => URL.revokeObjectURL(url));
}

/**
 * Decodes a photo the right way up (its EXIF orientation applied):
 * `createImageBitmap` where it takes `imageOrientation: "from-image"`, else an
 * `<img>` from a `blob:` URL, which browsers draw upright. Rejects when the
 * file is not a picture the browser can read.
 */
export async function loadForScan(file: File): Promise<{ bitmap: ScanImage; width: number; height: number }> {
  if (typeof createImageBitmap === "function") {
    try {
      const bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
      if (bitmap.width > 0 && bitmap.height > 0) return { bitmap, width: bitmap.width, height: bitmap.height };
      bitmap.close();
    } catch {
      // An older engine without the option, or a format only <img> decodes.
    }
  }
  const img = await loadImage(file);
  const width = img.naturalWidth || img.width;
  const height = img.naturalHeight || img.height;
  if (!width || !height) throw new Error("cover scan: the photo could not be read");
  return { bitmap: img, width, height };
}

/**
 * The photo's pixels, scaled down so that its long side is at most `maxSide`
 * (never up). `scale` is the factor applied: divide coordinates found in
 * `data` by it to get the photo's.
 */
export function readPixels(bitmap: ScanImage, width: number, height: number, maxSide: number): { data: ImageData; scale: number } {
  const scale = Math.min(1, maxSide / Math.max(width, height));
  const w = Math.max(1, Math.round(width * scale));
  const h = Math.max(1, Math.round(height * scale));
  const { ctx } = canvas(w, h);
  ctx.drawImage(bitmap, 0, 0, w, h);
  return { data: ctx.getImageData(0, 0, w, h), scale };
}

function toJpeg(el: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    el.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("cover scan: the browser could not encode the cover"))), "image/jpeg", JPEG_QUALITY);
  });
}

/**
 * Cuts the cover inside `quad` (photo pixels, in `Quad` order) out of the
 * photo and flattens it into an upright JPEG `cover.jpg` of `outputSize()`
 * (long side at most 1600 px). Only the quad's bounding box is drawn, scaled
 * to about the output's resolution, so a 12 MP photo needs no 12 MP canvas.
 * Throws a `RangeError` when the corners do not form a convex quad.
 */
export async function flatten(bitmap: ScanImage, width: number, height: number, quad: Quad): Promise<File> {
  if (!isConvex(quad)) throw new RangeError("cover scan: the corners do not form a convex quadrilateral");
  const out = outputSize(quad, MAX_OUTPUT_SIDE);

  // The part of the photo the quad covers, clamped to the photo.
  const left = Math.max(0, Math.floor(Math.min(...quad.map((p) => p.x))));
  const top = Math.max(0, Math.floor(Math.min(...quad.map((p) => p.y))));
  const right = Math.min(width, Math.ceil(Math.max(...quad.map((p) => p.x))));
  const bottom = Math.min(height, Math.ceil(Math.max(...quad.map((p) => p.y))));
  const boxW = Math.max(1, right - left);
  const boxH = Math.max(1, bottom - top);

  // Sample the box at about the output's resolution: no finer (it would only
  // cost memory), no coarser (the warp would blur), within the canvas limit.
  const sides = quadSides(quad);
  const quadLong = Math.max(sides.width, sides.height, 1);
  const ratio = Math.min(1, Math.max(out.width, out.height) / quadLong, Math.sqrt(MAX_CANVAS_PIXELS / (boxW * boxH)));
  const workW = Math.max(1, Math.round(boxW * ratio));
  const workH = Math.max(1, Math.round(boxH * ratio));
  const work = canvas(workW, workH);
  work.ctx.drawImage(bitmap, left, top, boxW, boxH, 0, 0, workW, workH);
  const pixels = work.ctx.getImageData(0, 0, workW, workH);
  const sx = workW / boxW;
  const sy = workH / boxH;
  const local = quad.map((p) => ({ x: (p.x - left) * sx, y: (p.y - top) * sy })) as Quad;

  const flat = warpQuad(pixels, local, out.width, out.height);
  const result = canvas(out.width, out.height);
  result.ctx.putImageData(flat, 0, 0);
  const blob = await toJpeg(result.canvas);
  return new File([blob], "cover.jpg", { type: "image/jpeg" });
}
