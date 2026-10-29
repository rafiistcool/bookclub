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

/** The rear camera if there is one, at about 720p, shown in `video` (`playsinline muted autoplay`). */
export async function openCamera(video: HTMLVideoElement): Promise<MediaStream> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      facingMode: { ideal: "environment" },
      width: { ideal: 1280 },
      height: { ideal: 720 },
    },
  });
  try {
    video.srcObject = stream;
    await video.play();
  } catch (err) {
    stopCamera(stream);
    video.srcObject = null;
    throw err;
  }
  return stream;
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
