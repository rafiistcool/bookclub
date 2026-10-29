import { beforeEach, describe, expect, it, vi } from "vitest";

const prepareZXingModule = vi.fn();
const purgeZXingModule = vi.fn();
const readBarcodes = vi.fn();

vi.mock("zxing-wasm/reader", () => ({
  prepareZXingModule: (...args: unknown[]) => prepareZXingModule(...args),
  purgeZXingModule: (...args: unknown[]) => purgeZXingModule(...args),
  readBarcodes: (...args: unknown[]) => readBarcodes(...args),
}));

type Overrides = { locateFile: (path: string, prefix: string) => string };
type Prepared = { overrides: Overrides; fireImmediately: boolean };

/** A fresh copy of the module, so its "prepared once" state starts empty. */
async function zxing() {
  vi.resetModules();
  return import("./zxing");
}

function canvas(image: unknown = { width: 4, height: 2, data: new Uint8ClampedArray(32) }) {
  const getImageData = vi.fn(() => image);
  const getContext = vi.fn(() => ({ getImageData }));
  return { element: { width: 4, height: 2, getContext } as unknown as HTMLCanvasElement, getContext, getImageData };
}

describe("the zxing decoder", () => {
  beforeEach(() => {
    prepareZXingModule.mockReset().mockResolvedValue({});
    purgeZXingModule.mockReset();
    readBarcodes.mockReset().mockResolvedValue([]);
  });

  it("prepares the module with our own wasm before the first read", async () => {
    const { createZxingDecoder } = await zxing();
    await createZxingDecoder();
    expect(prepareZXingModule).toHaveBeenCalledTimes(1);
    const options = prepareZXingModule.mock.calls[0][0] as Prepared;
    expect(options.fireImmediately).toBe(true);
    expect(readBarcodes).not.toHaveBeenCalled();

    // The package's own default would answer with fastly.jsdelivr.net here.
    const url = options.overrides.locateFile("zxing_reader.wasm", "https://fastly.jsdelivr.net/npm/zxing-wasm@3.1.6/dist/reader/");
    expect(url.startsWith("/")).toBe(true);
    expect(url).not.toContain("://");
    expect(url.startsWith("//")).toBe(false);
    expect(url).toMatch(/zxing_reader.*\.wasm$/);
    // Anything else stays where Emscripten looks for it.
    expect(options.overrides.locateFile("zxing_reader.data", "/x/")).toBe("/x/zxing_reader.data");
  });

  it("prepares once per page and reuses the same overrides", async () => {
    const { createZxingDecoder } = await zxing();
    await createZxingDecoder();
    await createZxingDecoder();
    expect(prepareZXingModule).toHaveBeenCalledTimes(1);
  });

  it("purges a module that failed to start, so the next scan tries again", async () => {
    const { createZxingDecoder } = await zxing();
    prepareZXingModule.mockRejectedValueOnce(new WebAssembly.CompileError("refused"));
    await expect(createZxingDecoder()).rejects.toBeInstanceOf(WebAssembly.CompileError);
    expect(purgeZXingModule).toHaveBeenCalledTimes(1);
    await createZxingDecoder();
    expect(prepareZXingModule).toHaveBeenCalledTimes(2);
  });

  it("reads one EAN-13 from the canvas and keeps only valid results", async () => {
    const { createZxingDecoder } = await zxing();
    const decoder = await createZxingDecoder();
    expect(decoder.kind).toBe("zxing");
    readBarcodes.mockResolvedValue([
      { isValid: false, text: "9780316769487" },
      { isValid: true, text: "9780316769488" },
    ]);
    const { element, getContext, getImageData } = canvas();
    await expect(decoder.detect(element)).resolves.toEqual(["9780316769488"]);
    expect(getContext).toHaveBeenCalledWith("2d", { willReadFrequently: true });
    expect(getImageData).toHaveBeenCalledWith(0, 0, 4, 2);
    expect(readBarcodes).toHaveBeenCalledWith(getImageData.mock.results[0].value, {
      formats: ["EAN13"],
      tryHarder: true,
      maxNumberOfSymbols: 1,
    });
  });
});
