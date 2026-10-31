/**
 * The zxing-wasm decoder (MIT; its wasm bundles zxing-cpp, Apache-2.0), loaded
 * only through `createDecoder`'s dynamic import.
 *
 * zxing-wasm fetches its wasm from fastly.jsdelivr.net unless told otherwise.
 * The shell's CSP (`connect-src 'self'`) blocks that, and the browser contacts
 * no third party (`thirdParty.test.ts`), so the module is always prepared with
 * our `locateFile` first: Vite emits the wasm as `/assets/zxing_reader-<hash>.wasm`,
 * and the server serves it as `application/wasm`, same-origin and immutable.
 */
import { prepareZXingModule, purgeZXingModule, readBarcodes } from "zxing-wasm/reader";
import wasmUrl from "zxing-wasm/reader/zxing_reader.wasm?url";
import type { Decoder } from "./decoder";

/** Every `.wasm` from our own build output; anything else where Emscripten looks for it. */
export function locateFile(path: string, prefix: string): string {
  return path.endsWith(".wasm") ? wasmUrl : prefix + path;
}

// One object, so zxing-wasm's shallow comparison keeps reusing the prepared module.
const OVERRIDES = { locateFile };

let prepared: Promise<unknown> | null = null;

/** Instantiates the module once, before the first read (a read before it would use the CDN). */
function prepare(): Promise<unknown> {
  prepared ??= prepareZXingModule({ overrides: OVERRIDES, fireImmediately: true }).catch((err: unknown) => {
    // A failed start (no WebAssembly, the CSP, a network error) is not kept: a later scan tries again.
    prepared = null;
    purgeZXingModule();
    throw err;
  });
  return prepared;
}

export async function createZxingDecoder(): Promise<Decoder> {
  await prepare();
  return {
    kind: "zxing",
    async detect(canvas) {
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context || canvas.width === 0 || canvas.height === 0) return [];
      const image = context.getImageData(0, 0, canvas.width, canvas.height);
      const results = await readBarcodes(image, {
        formats: ["EAN13"],
        tryHarder: true,
        maxNumberOfSymbols: 1,
      });
      return results.filter((result) => result.isValid).map((result) => result.text);
    },
  };
}
