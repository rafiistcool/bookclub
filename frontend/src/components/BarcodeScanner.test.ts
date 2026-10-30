import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const cameraSupport = vi.fn();
const openCamera = vi.fn();
const showCamera = vi.fn();
const createDecoder = vi.fn();
const drawScanBand = vi.fn();
const detect = vi.fn();

vi.mock("../scan/camera", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../scan/camera")>()),
  cameraSupport: () => cameraSupport(),
  openCamera: (...args: unknown[]) => openCamera(...args),
  showCamera: (...args: unknown[]) => showCamera(...args),
}));

vi.mock("../scan/decoder", () => ({
  createDecoder: () => createDecoder(),
  drawScanBand: (...args: unknown[]) => drawScanBand(...args),
}));

import BarcodeScanner from "./BarcodeScanner.vue";

/** A stream whose tracks record `stop()`. */
function fakeStream() {
  const track = { stop: vi.fn() };
  return { track, stream: { getTracks: () => [track] } as unknown as MediaStream };
}

/**
 * `<video>` as browsers have it: `play()` waits until `startPlaying()`, and a new `srcObject`
 * interrupts a pending `play()` with `AbortError`. jsdom implements neither.
 */
function browserVideo() {
  const proto = HTMLMediaElement.prototype;
  const pending = new Map<HTMLMediaElement, { resolve: () => void; reject: (err: DOMException) => void }>();
  const sources = new WeakMap<HTMLMediaElement, MediaStream | null>();
  Object.defineProperty(proto, "srcObject", {
    configurable: true,
    get(this: HTMLMediaElement) {
      return sources.get(this) ?? null;
    },
    set(this: HTMLMediaElement, value: MediaStream | null) {
      pending.get(this)?.reject(new DOMException("interrupted by a new load request", "AbortError"));
      pending.delete(this);
      sources.set(this, value);
    },
  });
  Object.defineProperty(proto, "play", {
    configurable: true,
    value(this: HTMLMediaElement) {
      return new Promise<void>((resolve, reject) => pending.set(this, { resolve, reject }));
    },
  });
  return {
    startPlaying() {
      for (const play of pending.values()) play.resolve();
      pending.clear();
    },
    restore() {
      delete (proto as unknown as Record<string, unknown>).srcObject;
      delete (proto as unknown as Record<string, unknown>).play;
    },
  };
}

function setVisibility(value: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => value });
  document.dispatchEvent(new Event("visibilitychange"));
}

let wrapper: VueWrapper | null = null;

async function mountScanner() {
  wrapper = mount(BarcodeScanner, { attachTo: document.body });
  await flushPromises();
  return wrapper;
}

/** One decode pass: the 200 ms timer, then the detect promise. */
async function pass() {
  await vi.advanceTimersByTimeAsync(200);
  await flushPromises();
}

describe("BarcodeScanner", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    cameraSupport.mockReset().mockReturnValue("ok");
    openCamera.mockReset();
    showCamera.mockReset().mockResolvedValue(undefined);
    detect.mockReset().mockResolvedValue([]);
    createDecoder.mockReset().mockResolvedValue({ kind: "zxing", detect });
    drawScanBand.mockReset().mockReturnValue(true);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
  });

  function manualEntry(w: VueWrapper) {
    return w.find("input[name='isbn']");
  }

  it("shows manual entry, focused, without a secure connection", async () => {
    cameraSupport.mockReturnValue("insecure");
    const w = await mountScanner();
    expect(w.text()).toContain("The camera needs a secure connection.");
    expect(manualEntry(w).exists()).toBe(true);
    expect(document.activeElement).toBe(manualEntry(w).element);
    expect(openCamera).not.toHaveBeenCalled();
    expect(createDecoder).not.toHaveBeenCalled();
  });

  it("says there is no camera, with manual entry, when the browser has none", async () => {
    cameraSupport.mockReturnValue("unsupported");
    const w = await mountScanner();
    expect(w.text()).toContain("No camera here.");
    expect(manualEntry(w).exists()).toBe(true);
  });

  it("says there is no camera when getUserMedia finds none", async () => {
    openCamera.mockRejectedValue(new DOMException("none", "NotFoundError"));
    const w = await mountScanner();
    expect(w.text()).toContain("No camera here.");
    expect(manualEntry(w).exists()).toBe(true);
    expect(w.find("video").exists()).toBe(false);
  });

  it("explains where to allow a denied camera and tries again", async () => {
    openCamera.mockRejectedValueOnce(new DOMException("no", "NotAllowedError"));
    const w = await mountScanner();
    expect(w.text()).toContain("The camera is blocked for this site.");
    expect(w.text()).toContain("Settings → Safari → Camera");
    expect(w.text()).toContain("Permissions → Camera");
    expect(manualEntry(w).exists()).toBe(true);
    const { stream } = fakeStream();
    openCamera.mockResolvedValueOnce(stream);
    await w.findAll("button").find((button) => button.text() === "Try again")!.trigger("click");
    await flushPromises();
    expect(openCamera).toHaveBeenCalledTimes(2);
    expect(w.text()).toContain("Hold the barcode 15–20 cm away");
  });

  it("says another app has the camera, with Try again and manual entry", async () => {
    openCamera.mockRejectedValue(new DOMException("busy", "NotReadableError"));
    const w = await mountScanner();
    expect(w.text()).toContain("The camera is in use by another app.");
    expect(w.findAll("button").some((button) => button.text() === "Try again")).toBe(true);
    expect(manualEntry(w).exists()).toBe(true);
  });

  it("says another app has the camera when the video cannot start, and tries again", async () => {
    const first = fakeStream();
    const second = fakeStream();
    openCamera.mockResolvedValueOnce(first.stream).mockResolvedValueOnce(second.stream);
    showCamera.mockRejectedValueOnce(new DOMException("busy", "NotReadableError"));
    const w = await mountScanner();
    expect(showCamera).toHaveBeenCalledWith(expect.any(HTMLVideoElement), first.stream);
    expect(w.text()).toContain("The camera is in use by another app.");
    await w.findAll("button").find((button) => button.text() === "Try again")!.trigger("click");
    await flushPromises();
    expect(showCamera).toHaveBeenLastCalledWith(expect.any(HTMLVideoElement), second.stream);
    expect(w.text()).toContain("Hold the barcode 15–20 cm away");
  });

  it("falls back to manual entry when no decoder can run, and stops the camera", async () => {
    const { stream, track } = fakeStream();
    openCamera.mockResolvedValue(stream);
    createDecoder.mockRejectedValue(new WebAssembly.CompileError("refused"));
    const w = await mountScanner();
    expect(w.text()).toContain("This browser cannot read barcodes.");
    expect(track.stop).toHaveBeenCalled();
    expect(manualEntry(w).exists()).toBe(true);
  });

  it("emits the ISBN-13 of a valid typed ISBN-10", async () => {
    cameraSupport.mockReturnValue("insecure");
    const w = await mountScanner();
    await manualEntry(w).setValue("3-518-36881-8");
    await w.find("form").trigger("submit");
    expect(w.emitted("isbn")).toEqual([["9783518368817"]]);
  });

  it("shows the invalid-ISBN copy for a wrong typed ISBN and emits nothing", async () => {
    cameraSupport.mockReturnValue("insecure");
    const w = await mountScanner();
    await manualEntry(w).setValue("978-0-316-76948-7");
    await w.find("form").trigger("submit");
    expect(w.text()).toContain("That is not a valid ISBN-10 or ISBN-13");
    expect(w.emitted("isbn")).toBeUndefined();
  });

  it("ignores a 977 EAN and a bad check digit, and emits the first ISBN once", async () => {
    const { stream, track } = fakeStream();
    openCamera.mockResolvedValue(stream);
    detect
      .mockResolvedValueOnce(["9771234567003"])
      .mockResolvedValueOnce(["9780316769487"])
      .mockResolvedValueOnce(["9780316769488"])
      .mockResolvedValue(["9791090636071"]);
    const w = await mountScanner();
    expect(w.find("video").exists()).toBe(true);
    await pass();
    await pass();
    expect(w.emitted("isbn")).toBeUndefined();
    expect(track.stop).not.toHaveBeenCalled();
    await pass();
    expect(w.emitted("isbn")).toEqual([["9780316769488"]]);
    expect(track.stop).toHaveBeenCalledTimes(1);
    await pass();
    await pass();
    expect(detect).toHaveBeenCalledTimes(3);
    expect(w.emitted("isbn")).toHaveLength(1);
  });

  it("never starts a pass before the last one ended", async () => {
    const { stream } = fakeStream();
    openCamera.mockResolvedValue(stream);
    let release!: (texts: string[]) => void;
    detect.mockImplementationOnce(() => new Promise<string[]>((resolve) => (release = resolve)));
    await mountScanner();
    await pass();
    await pass();
    await pass();
    expect(detect).toHaveBeenCalledTimes(1);
    release([]);
    await flushPromises();
    await pass();
    expect(detect).toHaveBeenCalledTimes(2);
  });

  it("stops the tracks on unmount", async () => {
    const { stream, track } = fakeStream();
    openCamera.mockResolvedValue(stream);
    const w = await mountScanner();
    w.unmount();
    wrapper = null;
    expect(track.stop).toHaveBeenCalledTimes(1);
    await pass();
    expect(detect).not.toHaveBeenCalled();
  });

  it("stops a stream that opens after unmount", async () => {
    const { stream, track } = fakeStream();
    let resolve!: (value: MediaStream) => void;
    openCamera.mockReturnValue(new Promise<MediaStream>((r) => (resolve = r)));
    const w = await mountScanner();
    w.unmount();
    wrapper = null;
    resolve(stream);
    await flushPromises();
    expect(track.stop).toHaveBeenCalledTimes(1);
  });

  it("stops the tracks when the page is hidden and starts again when it is back", async () => {
    const first = fakeStream();
    const second = fakeStream();
    openCamera.mockResolvedValueOnce(first.stream).mockResolvedValueOnce(second.stream);
    await mountScanner();
    setVisibility("hidden");
    expect(first.track.stop).toHaveBeenCalledTimes(1);
    await pass();
    expect(detect).not.toHaveBeenCalled();
    setVisibility("visible");
    await flushPromises();
    expect(openCamera).toHaveBeenCalledTimes(2);
    expect(second.track.stop).not.toHaveBeenCalled();
    await pass();
    expect(detect).toHaveBeenCalledTimes(1);
  });

  it("keeps one stream when the page comes back while the permission prompt is open", async () => {
    // The real camera module, a getUserMedia that waits for the prompt, and a browser's <video>.
    const actual = await vi.importActual<typeof import("../scan/camera")>("../scan/camera");
    openCamera.mockImplementation(actual.openCamera);
    showCamera.mockImplementation(actual.showCamera);
    const prompts: Array<(stream: MediaStream) => void> = [];
    vi.stubGlobal("navigator", {
      mediaDevices: { getUserMedia: () => new Promise<MediaStream>((allow) => prompts.push(allow)) },
    });
    const video = browserVideo();
    try {
      const first = fakeStream();
      const second = fakeStream();
      const w = await mountScanner();
      expect(prompts).toHaveLength(1);
      setVisibility("hidden");
      setVisibility("visible");
      await flushPromises();
      expect(prompts).toHaveLength(2);

      // The reader allows: both requests resolve.
      prompts[0](first.stream);
      prompts[1](second.stream);
      await flushPromises();
      video.startPlaying();
      await flushPromises();

      expect(w.text()).not.toContain("The camera is in use by another app.");
      expect(w.text()).toContain("Hold the barcode 15–20 cm away");
      expect(first.track.stop).toHaveBeenCalledTimes(1);
      expect(second.track.stop).not.toHaveBeenCalled();
      expect(w.get("video").element.srcObject).toBe(second.stream);
      await pass();
      expect(detect).toHaveBeenCalledTimes(1);
    } finally {
      video.restore();
    }
  });

  it("stops the camera and emits cancel", async () => {
    const { stream, track } = fakeStream();
    openCamera.mockResolvedValue(stream);
    const w = await mountScanner();
    await w.findAll("button").find((button) => button.text() === "Cancel")!.trigger("click");
    expect(track.stop).toHaveBeenCalledTimes(1);
    expect(w.emitted("cancel")).toHaveLength(1);
  });
});
