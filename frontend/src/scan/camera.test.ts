import { afterEach, describe, expect, it, vi } from "vitest";
import { cameraError, cameraSupport, openCamera, stopCamera } from "./camera";

function fakeStream() {
  const tracks = [{ stop: vi.fn() }, { stop: vi.fn() }];
  return { tracks, stream: { getTracks: () => tracks } as unknown as MediaStream };
}

function fakeVideo(play: () => Promise<void> = async () => undefined) {
  return { srcObject: null as MediaStream | null, play: vi.fn(play) };
}

describe("cameraSupport", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is insecure outside a secure context, whatever mediaDevices says", () => {
    vi.stubGlobal("isSecureContext", false);
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn() } });
    expect(cameraSupport()).toBe("insecure");
  });

  it("is unsupported without mediaDevices or getUserMedia", () => {
    vi.stubGlobal("isSecureContext", true);
    vi.stubGlobal("navigator", {});
    expect(cameraSupport()).toBe("unsupported");
    vi.stubGlobal("navigator", { mediaDevices: {} });
    expect(cameraSupport()).toBe("unsupported");
  });

  it("is ok in a secure context with getUserMedia", () => {
    vi.stubGlobal("isSecureContext", true);
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn() } });
    expect(cameraSupport()).toBe("ok");
  });
});

describe("cameraError", () => {
  it.each([
    ["NotAllowedError", "denied"],
    ["SecurityError", "denied"],
    ["NotFoundError", "nocamera"],
    ["OverconstrainedError", "nocamera"],
    ["NotReadableError", "busy"],
    ["AbortError", "busy"],
    ["TypeError", "failed"],
    ["InvalidStateError", "failed"],
  ])("maps %s to %s", (name, problem) => {
    expect(cameraError(new DOMException("camera", name))).toBe(problem);
  });

  it("reads the name of a plain error object and fails on anything else", () => {
    // Chrome's OverconstrainedError is not a DOMException.
    expect(cameraError({ name: "OverconstrainedError", constraint: "facingMode" })).toBe("nocamera");
    expect(cameraError(new Error("boom"))).toBe("failed");
    expect(cameraError("NotAllowedError")).toBe("failed");
    expect(cameraError(null)).toBe("failed");
  });
});

describe("openCamera and stopCamera", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("asks for the rear camera without audio and shows the stream", async () => {
    const { stream } = fakeStream();
    const getUserMedia = vi.fn().mockResolvedValue(stream);
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia } });
    const video = fakeVideo();
    await expect(openCamera(video as unknown as HTMLVideoElement)).resolves.toBe(stream);
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: false,
      video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 }, height: { ideal: 720 } },
    });
    expect(video.srcObject).toBe(stream);
    expect(video.play).toHaveBeenCalled();
  });

  it("stops the stream when the video cannot play", async () => {
    const { stream, tracks } = fakeStream();
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(stream) } });
    const video = fakeVideo(() => Promise.reject(new DOMException("gone", "AbortError")));
    await expect(openCamera(video as unknown as HTMLVideoElement)).rejects.toMatchObject({ name: "AbortError" });
    expect(tracks.every((track) => track.stop.mock.calls.length === 1)).toBe(true);
    expect(video.srcObject).toBeNull();
  });

  it("stops every track, and nothing without a stream", () => {
    const { stream, tracks } = fakeStream();
    stopCamera(stream);
    expect(tracks.map((track) => track.stop.mock.calls.length)).toEqual([1, 1]);
    expect(() => stopCamera(null)).not.toThrow();
  });
});
