import { describe, expect, it, vi } from "vitest";

/**
 * `public/sw.js` replaces the Python app's offline worker, which installed
 * apps from before PR #3 may still run (#52, risk R22). The browser's update
 * check of `/sw.js` gets this file; it must clear the old caches and
 * unregister, and must never answer a request itself.
 */

type Fs = { readFileSync(path: URL, encoding: "utf8"): string };

/** The frontend directory (a variable: Vite rewrites `new URL` of a literal). */
const ROOT = "../";

async function read(path: string): Promise<string> {
  const fs = await vi.importActual<Fs>("node:fs");
  return fs.readFileSync(new URL(ROOT + path, import.meta.url), "utf8");
}

/** The script without its leading block comment. */
function code(script: string): string {
  const end = script.indexOf("*/");
  expect(script.startsWith("/*")).toBe(true);
  return script.slice(end + 2).trim();
}

type Listener = (event: { waitUntil(promise: Promise<unknown>): void }) => void;

/** Runs the worker against stand-ins for its global scope. */
function runWorker(source: string, cacheKeys: string[], windows: string[]) {
  const listeners = new Map<string, Listener>();
  const calls: string[] = [];
  const scope = {
    addEventListener: (type: string, listener: Listener) => listeners.set(type, listener),
    skipWaiting: () => calls.push("skipWaiting"),
    clients: {
      claim: async () => calls.push("claim"),
      matchAll: async (options: { type: string }) => {
        calls.push(`matchAll ${options.type}`);
        return windows.map((url) => ({
          url,
          navigate: async (to: string) => calls.push(`navigate ${to}`),
        }));
      },
    },
    registration: { unregister: async () => calls.push("unregister") },
  };
  const caches = {
    keys: async () => cacheKeys,
    delete: async (key: string) => calls.push(`delete ${key}`),
  };
  new Function("self", "caches", source)(scope, caches);
  return { listeners, calls };
}

describe("the service worker kill switch", () => {
  it("unregisters, deletes the caches and has no fetch listener", async () => {
    const script = await read("public/sw.js");
    expect(script).toContain("self.registration.unregister()");
    expect(script).toContain("caches.delete");
    expect(script).not.toMatch(/addEventListener\(\s*["'`]fetch["'`]/);
    expect(script).not.toMatch(/\bonfetch\b/);
    expect(script).not.toMatch(/\bimportScripts\b/);
  });

  it("is the old hostname's worker with its own header comment", async () => {
    const app = await read("public/sw.js");
    const legacyOrigin = await read("../deploy/legacy-origin/sw.js");
    expect(code(app)).toBe(code(legacyOrigin));
  });

  it("clears every cache, unregisters and reloads the open pages", async () => {
    const source = await read("public/sw.js");
    const { listeners, calls } = runWorker(
      source,
      ["bookclub-shell-v3", "bookclub-covers-v1", "bookclub-data-v2"],
      ["https://bookclub.example/shelf", "https://bookclub.example/club/x"],
    );
    expect([...listeners.keys()].sort()).toEqual(["activate", "install"]);

    listeners.get("install")!({ waitUntil: () => undefined });
    expect(calls).toEqual(["skipWaiting"]);

    let done: Promise<unknown> = Promise.resolve();
    listeners.get("activate")!({ waitUntil: (promise) => (done = promise) });
    await done;
    expect(calls).toEqual([
      "skipWaiting",
      "claim",
      "delete bookclub-shell-v3",
      "delete bookclub-covers-v1",
      "delete bookclub-data-v2",
      "unregister",
      "matchAll window",
      "navigate https://bookclub.example/shelf",
      "navigate https://bookclub.example/club/x",
    ]);
  });
});
