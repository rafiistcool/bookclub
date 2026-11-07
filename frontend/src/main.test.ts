import { describe, expect, it, vi } from "vitest";

type Fs = { readFileSync(path: URL, encoding: "utf8"): string };

/** The file to read (a variable: Vite rewrites `new URL` of a literal). */
const MAIN = "./main.ts";

describe("main.ts boot", () => {
  it("never awaits the first navigation at the top level", async () => {
    const fs = await vi.importActual<Fs>("node:fs");
    const source = fs.readFileSync(new URL(MAIN, import.meta.url), "utf8");
    // The first page's lazy chunk imports from the entry module, and a dynamic
    // import waits until the entry has finished evaluating: a top-level await
    // on the router would wait on itself, and the app would never mount.
    expect(source).not.toMatch(/^await\s+router\b/m);
  });
});
