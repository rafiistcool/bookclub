import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { insetQuad, outputSize, type Quad } from "./geometry";
import { flatten, JPEG_QUALITY, loadForScan, readPixels, type ScanImage } from "./index";

// jsdom has no canvas: a 2D context that records its calls and hands back
// pixels of one colour, and a canvas that "encodes" a tiny JPEG.
interface FakeContext {
  canvas: HTMLCanvasElement;
  drawImage: ReturnType<typeof vi.fn>;
  getImageData: ReturnType<typeof vi.fn>;
  putImageData: ReturnType<typeof vi.fn>;
  imageSmoothingEnabled: boolean;
  imageSmoothingQuality: string;
}

let contexts: FakeContext[];
let blobs: { type: string; quality: number }[];
const FILL = [12, 34, 56, 255];

beforeEach(() => {
  contexts = [];
  blobs = [];
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function (this: HTMLCanvasElement) {
    const canvas = this;
    const ctx: FakeContext = {
      canvas,
      drawImage: vi.fn(),
      getImageData: vi.fn((_x: number, _y: number, w: number, h: number) => {
        const data = new Uint8ClampedArray(w * h * 4);
        for (let i = 0; i < data.length; i += 4) data.set(FILL, i);
        return { data, width: w, height: h };
      }),
      putImageData: vi.fn(),
      imageSmoothingEnabled: false,
      imageSmoothingQuality: "low",
    };
    contexts.push(ctx);
    return ctx as unknown as CanvasRenderingContext2D;
  } as unknown as typeof HTMLCanvasElement.prototype.getContext);
  vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation(function (callback: BlobCallback, type?: string, quality?: number) {
    blobs.push({ type: type ?? "", quality: quality ?? -1 });
    callback(new Blob([new Uint8Array([0xff, 0xd8, 0xff, 0xd9])], { type }));
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const bitmap = { width: 4000, height: 3000 } as unknown as ScanImage;

describe("readPixels", () => {
  it("scales the photo down to the long side asked for", () => {
    const { data, scale } = readPixels(bitmap, 4000, 3000, 400);
    expect(scale).toBe(0.1);
    expect(data.width).toBe(400);
    expect(data.height).toBe(300);
    const [ctx] = contexts;
    expect(ctx.canvas.width).toBe(400);
    expect(ctx.canvas.height).toBe(300);
    expect(ctx.drawImage).toHaveBeenCalledWith(bitmap, 0, 0, 400, 300);
    expect(ctx.imageSmoothingQuality).toBe("high");
  });

  it("scales a portrait photo by its height", () => {
    const { data, scale } = readPixels(bitmap, 3024, 4032, 512);
    expect(scale).toBeCloseTo(512 / 4032, 9);
    expect(data.width).toBe(384);
    expect(data.height).toBe(512);
  });

  it("never scales up", () => {
    const { data, scale } = readPixels(bitmap, 300, 200, 512);
    expect(scale).toBe(1);
    expect(data.width).toBe(300);
    expect(data.height).toBe(200);
  });

  it("fails clearly without a 2D canvas", () => {
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);
    expect(() => readPixels(bitmap, 300, 200, 512)).toThrow(/canvas/);
  });
});

describe("flatten", () => {
  const quad: Quad = [
    { x: 1200.4, y: 600.2 },
    { x: 2500, y: 700 },
    { x: 2700, y: 2600.6 },
    { x: 1000, y: 2500 },
  ];

  it("returns an upright JPEG named cover.jpg of the output size", async () => {
    const file = await flatten(bitmap, 4000, 3000, quad);
    expect(file).toBeInstanceOf(File);
    expect(file.name).toBe("cover.jpg");
    expect(file.type).toBe("image/jpeg");
    expect(blobs).toEqual([{ type: "image/jpeg", quality: JPEG_QUALITY }]);
    const size = outputSize(quad, 1600);
    const result = contexts[contexts.length - 1];
    expect(result.canvas.width).toBe(size.width);
    expect(result.canvas.height).toBe(size.height);
    const [flat, x, y] = result.putImageData.mock.calls[0];
    expect([x, y]).toEqual([0, 0]);
    expect(flat.width).toBe(size.width);
    expect(flat.height).toBe(size.height);
    // The warp read the working canvas's pixels, not the background.
    expect(Array.from(flat.data.slice(0, 4))).toEqual(FILL);
    expect(Array.from(flat.data.slice(-4))).toEqual(FILL);
  });

  it("draws only the quad's bounding box, at about the output's resolution", async () => {
    await flatten(bitmap, 4000, 3000, quad);
    const work = contexts[0];
    const [source, sx, sy, sw, sh, dx, dy, dw, dh] = work.drawImage.mock.calls[0];
    expect(source).toBe(bitmap);
    expect([sx, sy, sw, sh]).toEqual([1000, 600, 1700, 2001]);
    expect([dx, dy]).toEqual([0, 0]);
    // The quad is about 1500 × 1910 px; the output's long side is 1600.
    expect(dh).toBeLessThan(2001);
    expect(dh).toBeGreaterThan(1600);
    expect(dw / dh).toBeCloseTo(1700 / 2001, 2);
    expect(work.canvas.width).toBe(dw);
    expect(work.canvas.height).toBe(dh);
  });

  it("keeps a small cover at its own resolution", async () => {
    const small = insetQuad(800, 600, 0.25);
    await flatten(bitmap, 800, 600, small);
    const [, sx, sy, sw, sh, , , dw, dh] = contexts[0].drawImage.mock.calls[0];
    expect([sx, sy, sw, sh]).toEqual([200, 150, 400, 300]);
    expect([dw, dh]).toEqual([400, 300]);
  });

  it("clamps the box to the photo when a corner lies outside it", async () => {
    const out: Quad = [
      { x: -20, y: -10 },
      { x: 500, y: 0 },
      { x: 520, y: 700 },
      { x: 0, y: 690 },
    ];
    await flatten(bitmap, 500, 680, out);
    const [, sx, sy, sw, sh] = contexts[0].drawImage.mock.calls[0];
    expect([sx, sy, sw, sh]).toEqual([0, 0, 500, 680]);
  });

  it("refuses corners that do not form a convex quad", async () => {
    const [tl, tr, br, bl] = quad;
    await expect(flatten(bitmap, 4000, 3000, [tl, br, tr, bl])).rejects.toThrow(RangeError);
    expect(contexts).toHaveLength(0);
  });

  it("rejects when the browser cannot encode the JPEG", async () => {
    vi.mocked(HTMLCanvasElement.prototype.toBlob).mockImplementation((callback: BlobCallback) => callback(null));
    await expect(flatten(bitmap, 4000, 3000, quad)).rejects.toThrow(/encode/);
  });
});

describe("loadForScan", () => {
  const file = new File([new Uint8Array([1, 2, 3])], "IMG_0001.jpg", { type: "image/jpeg" });

  it("decodes with createImageBitmap and the photo's own orientation", async () => {
    const decoded = { width: 3024, height: 4032, close: vi.fn() };
    const create = vi.fn().mockResolvedValue(decoded);
    vi.stubGlobal("createImageBitmap", create);
    const result = await loadForScan(file);
    expect(create).toHaveBeenCalledWith(file, { imageOrientation: "from-image" });
    expect(result).toEqual({ bitmap: decoded, width: 3024, height: 4032 });
  });

  function stubImage(outcome: "load" | "error", width = 0, height = 0) {
    const urls = { created: [] as Blob[], revoked: [] as string[] };
    vi.stubGlobal("URL", Object.assign(Object.create(URL), {
      createObjectURL: (blob: Blob) => {
        urls.created.push(blob);
        return "blob:scan/1";
      },
      revokeObjectURL: (url: string) => urls.revoked.push(url),
    }));
    class FakeImage {
      naturalWidth = width;
      naturalHeight = height;
      width = width;
      height = height;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_url: string) {
        queueMicrotask(() => (outcome === "load" ? this.onload?.() : this.onerror?.()));
      }
    }
    vi.stubGlobal("Image", FakeImage);
    return urls;
  }

  it("falls back to an <img> when createImageBitmap refuses the option", async () => {
    vi.stubGlobal("createImageBitmap", vi.fn().mockRejectedValue(new TypeError("bad option")));
    const urls = stubImage("load", 1200, 1600);
    const result = await loadForScan(file);
    expect(result.width).toBe(1200);
    expect(result.height).toBe(1600);
    expect(urls.created).toEqual([file]);
    expect(urls.revoked).toEqual(["blob:scan/1"]);
  });

  it("falls back to an <img> without createImageBitmap", async () => {
    vi.stubGlobal("createImageBitmap", undefined);
    stubImage("load", 640, 480);
    expect((await loadForScan(file)).width).toBe(640);
  });

  it("closes an empty bitmap and falls back", async () => {
    const empty = { width: 0, height: 0, close: vi.fn() };
    vi.stubGlobal("createImageBitmap", vi.fn().mockResolvedValue(empty));
    stubImage("load", 10, 20);
    expect((await loadForScan(file)).height).toBe(20);
    expect(empty.close).toHaveBeenCalled();
  });

  it("rejects a file that is no picture, and frees its URL", async () => {
    vi.stubGlobal("createImageBitmap", vi.fn().mockRejectedValue(new DOMException("undecodable")));
    const urls = stubImage("error");
    await expect(loadForScan(file)).rejects.toThrow(/could not be read/);
    expect(urls.revoked).toEqual(["blob:scan/1"]);
  });

  it("rejects a picture without a size", async () => {
    vi.stubGlobal("createImageBitmap", undefined);
    stubImage("load", 0, 0);
    await expect(loadForScan(file)).rejects.toThrow(/could not be read/);
  });
});
