/**
 * Reads EAN-13 barcodes from a canvas (#56). The browser's own `BarcodeDetector`
 * when it can read EAN-13 (Android Chrome with Play Services), else zxing-wasm
 * (iOS Safari 16+, desktop browsers). zxing is a dynamic import, so neither its
 * glue nor its wasm is in the main bundle; `./zxing` loads the wasm from our own
 * `/assets` (the CSP allows WebAssembly from there, `'wasm-unsafe-eval'`).
 */

export type DecoderKind = "native" | "zxing";

export interface Decoder {
  kind: DecoderKind;
  /** The texts of the barcodes in the canvas, unchecked; the caller keeps the ISBNs. */
  detect(canvas: HTMLCanvasElement): Promise<string[]>;
}

// The DOM lib has no Barcode Detection API; these are the parts we use.
interface DetectedBarcode {
  rawValue: string;
}

interface BarcodeDetectorInstance {
  detect(source: HTMLCanvasElement): Promise<DetectedBarcode[]>;
}

interface BarcodeDetectorClass {
  new (options?: { formats?: string[] }): BarcodeDetectorInstance;
  getSupportedFormats(): Promise<string[]>;
}

async function nativeDetector(): Promise<Decoder | null> {
  const Native = (globalThis as { BarcodeDetector?: BarcodeDetectorClass }).BarcodeDetector;
  if (typeof Native !== "function") return null;
  let formats: string[];
  try {
    formats = await Native.getSupportedFormats();
  } catch {
    return null;
  }
  // Desktop Chrome on Linux has the class but lists no formats: `[]`.
  if (!Array.isArray(formats) || !formats.includes("ean_13")) return null;
  const detector = new Native({ formats: ["ean_13"] });
  return {
    kind: "native",
    async detect(canvas) {
      return (await detector.detect(canvas)).map((barcode) => barcode.rawValue);
    },
  };
}

/**
 * The decoder this browser can run. Rejects when neither works (no native EAN-13
 * and the wasm cannot load, as before iOS 16); the scanner then offers manual
 * entry only.
 */
export async function createDecoder(): Promise<Decoder> {
  const native = await nativeDetector();
  if (native) return native;
  const { createZxingDecoder } = await import("./zxing");
  return createZxingDecoder();
}

/** At most this many pixels wide: enough for a barcode across the frame, small enough for a phone. */
export const BAND_MAX_WIDTH = 1280;
/** The share of the frame's height that is decoded, centred: where the guide asks for the barcode. */
export const BAND_HEIGHT = 0.4;

export type Band = {
  /** The source rectangle in the video frame. */
  sx: number;
  sy: number;
  sw: number;
  sh: number;
  /** The canvas size it is drawn at. */
  width: number;
  height: number;
};

/** The centre band of a `width` × `height` frame: full width, the middle 40 %, scaled to at most 1280 px wide. */
export function scanBand(width: number, height: number): Band | null {
  if (!(width > 0 && height > 0)) return null;
  const sh = Math.max(1, Math.round(height * BAND_HEIGHT));
  const sy = Math.round((height - sh) / 2);
  const scale = Math.min(1, BAND_MAX_WIDTH / width);
  return {
    sx: 0,
    sy,
    sw: width,
    sh,
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(sh * scale)),
  };
}

/**
 * Draws the video's centre band into `canvas`. False while the video has no
 * frame yet. The context is created with `willReadFrequently`, since every
 * pass reads the pixels back (zxing), and it is the one `getContext` returns
 * afterwards.
 */
export function drawScanBand(video: HTMLVideoElement, canvas: HTMLCanvasElement): boolean {
  const band = scanBand(video.videoWidth, video.videoHeight);
  if (!band) return false;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return false;
  if (canvas.width !== band.width) canvas.width = band.width;
  if (canvas.height !== band.height) canvas.height = band.height;
  context.drawImage(video, band.sx, band.sy, band.sw, band.sh, 0, 0, band.width, band.height);
  return true;
}
