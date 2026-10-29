/**
 * The camera for the barcode scanner (#56). The frames go to the decoder in
 * this browser and nowhere else; only the ISBN it reads reaches the server.
 *
 * `getUserMedia` exists only in a secure context (HTTPS or `localhost`); on
 * `http://agents.lan:<port>` phones have no `navigator.mediaDevices` at all.
 * The proxy's `Permissions-Policy` must allow `camera=(self)`, or the browser
 * refuses with `NotAllowedError` without asking.
 */

export type CameraSupport = "ok" | "insecure" | "unsupported";

/** Why the camera did not start, by the `DOMException` name `getUserMedia` or `play()` threw. */
export type CameraProblem = "denied" | "nocamera" | "busy" | "failed";

export function cameraSupport(): CameraSupport {
  if (globalThis.isSecureContext === false) return "insecure";
  if (typeof navigator === "undefined" || typeof navigator.mediaDevices?.getUserMedia !== "function") {
    return "unsupported";
  }
  return "ok";
}

/**
 * The rear camera if there is one, at about 720p. It waits for the permission prompt; the
 * caller checks it still wants the stream before `showCamera`, so a stale request never
 * touches the video (a new `srcObject` interrupts the `play()` of the stream before it).
 */
export function openCamera(): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      facingMode: { ideal: "environment" },
      width: { ideal: 1280 },
      height: { ideal: 720 },
    },
  });
}

/**
 * Shows `stream` in `video` (`playsinline muted autoplay`). If it cannot play, the stream is
 * stopped, and the video is cleared only while it still shows this stream.
 */
export async function showCamera(video: HTMLVideoElement, stream: MediaStream): Promise<void> {
  try {
    video.srcObject = stream;
    await video.play();
  } catch (err) {
    stopCamera(stream);
    if (video.srcObject === stream) video.srcObject = null;
    throw err;
  }
}

/** Stops every track, so the camera light goes off. */
export function stopCamera(stream: MediaStream | null | undefined): void {
  for (const track of stream?.getTracks() ?? []) track.stop();
}

export function cameraError(err: unknown): CameraProblem {
  const name = typeof err === "object" && err !== null ? (err as { name?: unknown }).name : undefined;
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return "denied";
    case "NotFoundError":
    case "OverconstrainedError":
      return "nocamera";
    case "NotReadableError":
    case "AbortError":
      return "busy";
    default:
      return "failed";
  }
}
