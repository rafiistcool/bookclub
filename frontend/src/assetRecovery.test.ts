import { afterEach, describe, expect, it, vi } from "vitest";
import { installAssetRecovery } from "./assetRecovery";

afterEach(() => vi.unstubAllGlobals());

function setup() {
  const events = new EventTarget();
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };
  const reload = vi.fn();
  vi.stubGlobal("window", {
    addEventListener: events.addEventListener.bind(events),
    sessionStorage: storage,
    location: { reload },
  });
  installAssetRecovery();
  return { events, reload, storage };
}

describe("asset recovery after deployment", () => {
  it("reloads once when an old page chunk is missing, without a reload loop", () => {
    const { events, reload } = setup();
    const first = new Event("vite:preloadError", { cancelable: true });
    events.dispatchEvent(first);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(first.defaultPrevented).toBe(true);

    // Simulate another app boot in the same browser tab after the reload.
    const nextBoot = new EventTarget();
    window.addEventListener = nextBoot.addEventListener.bind(nextBoot);
    installAssetRecovery();
    const second = new Event("vite:preloadError", { cancelable: true });
    nextBoot.dispatchEvent(second);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(second.defaultPrevented).toBe(false);
  });

  it("does not risk a reload loop when browser storage is blocked", () => {
    const { events, reload, storage } = setup();
    storage.setItem = () => { throw new Error("Storage blocked"); };
    events.dispatchEvent(new Event("vite:preloadError"));
    expect(reload).not.toHaveBeenCalled();
  });

  it("does not reload for ordinary API or application errors", () => {
    const { events, reload } = setup();
    events.dispatchEvent(new Event("error"));
    expect(reload).not.toHaveBeenCalled();
  });
});
