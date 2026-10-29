import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const createZxingDecoder = vi.fn();

vi.mock("./zxing", () => ({
  createZxingDecoder: (...args: unknown[]) => createZxingDecoder(...args),
}));

import { createDecoder, drawScanBand, scanBand } from "./decoder";

const canvas = {} as HTMLCanvasElement;

/** A stand-in for the browser's `BarcodeDetector` class. */
function nativeClass(formats: () => Promise<string[]>, values: string[] = []) {
  const constructed: unknown[] = [];
  class FakeDetector {
    static getSupportedFormats = formats;
    constructor(options?: unknown) {
      constructed.push(options);
    }
    async detect(source: unknown) {
      expect(source).toBe(canvas);
      return values.map((rawValue) => ({ rawValue, format: "ean_13" }));
    }
  }
  return { FakeDetector, constructed };
}

describe("createDecoder", () => {
  beforeEach(() => {
    createZxingDecoder.mockReset().mockResolvedValue({ kind: "zxing", detect: async () => [] });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("uses the native detector when it lists ean_13", async () => {
    const { FakeDetector, constructed } = nativeClass(async () => ["qr_code", "ean_13", "ean_8"], ["9780316769488"]);
    vi.stubGlobal("BarcodeDetector", FakeDetector);
    const decoder = await createDecoder();
    expect(decoder.kind).toBe("native");
    expect(constructed).toEqual([{ formats: ["ean_13"] }]);
    await expect(decoder.detect(canvas)).resolves.toEqual(["9780316769488"]);
    expect(createZxingDecoder).not.toHaveBeenCalled();
  });

  it("loads zxing when the native detector lists no formats", async () => {
    // Desktop Chrome on Linux: the class exists, the list is empty.
    const { FakeDetector, constructed } = nativeClass(async () => []);
    vi.stubGlobal("BarcodeDetector", FakeDetector);
    expect((await createDecoder()).kind).toBe("zxing");
    expect(constructed).toEqual([]);
    expect(createZxingDecoder).toHaveBeenCalledTimes(1);
  });

  it("loads zxing when the native detector cannot read EAN-13", async () => {
    vi.stubGlobal("BarcodeDetector", nativeClass(async () => ["qr_code"]).FakeDetector);
    expect((await createDecoder()).kind).toBe("zxing");
  });

  it("loads zxing when asking for the formats fails", async () => {
    vi.stubGlobal("BarcodeDetector", nativeClass(() => Promise.reject(new Error("no"))).FakeDetector);
    expect((await createDecoder()).kind).toBe("zxing");
  });

  it("loads zxing without a BarcodeDetector", async () => {
    vi.stubGlobal("BarcodeDetector", undefined);
    expect((await createDecoder()).kind).toBe("zxing");
    expect(createZxingDecoder).toHaveBeenCalledTimes(1);
  });

  it("rejects when zxing cannot start either", async () => {
    vi.stubGlobal("BarcodeDetector", undefined);
    createZxingDecoder.mockRejectedValue(new WebAssembly.CompileError("refused"));
    await expect(createDecoder()).rejects.toBeInstanceOf(WebAssembly.CompileError);
  });
});

describe("scanBand", () => {
  it("takes the full width and the middle 40 % of the height", () => {
    expect(scanBand(1280, 720)).toEqual({ sx: 0, sy: 216, sw: 1280, sh: 288, width: 1280, height: 288 });
    // A phone held upright: a tall frame.
    expect(scanBand(720, 1280)).toEqual({ sx: 0, sy: 384, sw: 720, sh: 512, width: 720, height: 512 });
  });

  it("scales a wider frame down to 1280 px", () => {
    expect(scanBand(1920, 1080)).toEqual({ sx: 0, sy: 324, sw: 1920, sh: 432, width: 1280, height: 288 });
  });

  it("has no band before the video has a frame", () => {
    expect(scanBand(0, 0)).toBeNull();
    expect(scanBand(640, 0)).toBeNull();
  });
});

describe("drawScanBand", () => {
  it("draws the band into a canvas of the band's size", () => {
    const drawImage = vi.fn();
    const getContext = vi.fn(() => ({ drawImage }));
    const target = { width: 300, height: 150, getContext } as unknown as HTMLCanvasElement;
    const video = { videoWidth: 1280, videoHeight: 720 } as HTMLVideoElement;
    expect(drawScanBand(video, target)).toBe(true);
    expect(getContext).toHaveBeenCalledWith("2d", { willReadFrequently: true });
    expect([target.width, target.height]).toEqual([1280, 288]);
    expect(drawImage).toHaveBeenCalledWith(video, 0, 216, 1280, 288, 0, 0, 1280, 288);
  });

  it("draws nothing while the video has no frame", () => {
    const getContext = vi.fn();
    const target = { getContext } as unknown as HTMLCanvasElement;
    expect(drawScanBand({ videoWidth: 0, videoHeight: 0 } as HTMLVideoElement, target)).toBe(false);
    expect(getContext).not.toHaveBeenCalled();
  });
});
