<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId } from "vue";
import { useI18n } from "vue-i18n";
import { isbnFromScan, parseIsbn } from "../isbn";
import { cameraError, cameraSupport, openCamera, showCamera, stopCamera } from "../scan/camera";
import { createDecoder, drawScanBand, type Decoder } from "../scan/decoder";

/**
 * The barcode scanner (#56): the camera, a decode every 200 ms of the centre
 * band, and manual entry, which is always there. The first EAN-13 that is an
 * ISBN (978/979, check digit) stops the camera and is emitted as `isbn`; a
 * typed ISBN-10 or ISBN-13 is emitted as its ISBN-13. Nothing is looked up here.
 *
 * Only one stream is ever open. The camera stops on a result, on cancel, on
 * unmount, and when the page is hidden (iOS ends the stream in the
 * background); it starts again when the page comes back.
 */
const emit = defineEmits<{ isbn: [isbn13: string]; cancel: [] }>();

type State =
  | "starting"
  | "live"
  | "insecure"
  | "unsupported"
  | "nocamera"
  | "denied"
  | "busy"
  | "failed"
  | "nodecoder";

const DECODE_EVERY_MS = 200;
/** Decode passes that may throw in a row before the decoder counts as broken. */
const MAX_DECODE_FAILURES = 5;

const { t } = useI18n();
const inputId = useId();
const state = ref<State>("starting");
const video = ref<HTMLVideoElement | null>(null);
const manualInput = ref<HTMLInputElement | null>(null);
const manual = ref("");
const manualError = ref("");

const showVideo = computed(() => state.value === "starting" || state.value === "live");
const canRetry = computed(() => ["denied", "busy", "failed"].includes(state.value));
const problem = computed(() => {
  switch (state.value) {
    case "insecure":
      return t("scan.insecure");
    case "unsupported":
    case "nocamera":
      return t("scan.noCamera");
    case "denied":
      return t("scan.denied");
    case "busy":
      return t("scan.busy");
    case "failed":
      return t("scan.failed");
    case "nodecoder":
      return t("scan.noDecoder");
    default:
      return "";
  }
});

let stream: MediaStream | null = null;
let decoder: Promise<Decoder> | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
/** Bumped by every stop; a start that finds it changed drops what it opened. */
let run = 0;
let done = false;
let failures = 0;
let resumeWhenVisible = false;
let canvas: HTMLCanvasElement | null = null;

function stop() {
  run += 1;
  if (timer !== null) clearTimeout(timer);
  timer = null;
  stopCamera(stream);
  stream = null;
  if (video.value) video.value.srcObject = null;
}

async function start() {
  stop();
  const current = run;
  const support = cameraSupport();
  if (support !== "ok") {
    state.value = support;
    if (support === "insecure") {
      await nextTick();
      manualInput.value?.focus();
    }
    return;
  }
  state.value = "starting";
  await nextTick();
  const element = video.value;
  if (current !== run || done || !element) return;
  if (!decoder) {
    // In parallel with the permission prompt. Marked as handled: it is awaited below.
    decoder = createDecoder();
    decoder.catch(() => undefined);
  }
  let opened: MediaStream;
  try {
    opened = await openCamera();
  } catch (err) {
    if (current === run) state.value = cameraError(err);
    return;
  }
  // Stopped while the prompt was open (hidden and back, unmounted): the newer start owns the
  // video, so this stream never reaches it. Its srcObject would interrupt the other's play().
  if (current !== run) {
    stopCamera(opened);
    return;
  }
  // From here on a stop() ends this stream, also while play() is pending.
  stream = opened;
  try {
    await showCamera(element, opened);
  } catch (err) {
    if (current !== run) return;
    stream = null;
    state.value = cameraError(err);
    return;
  }
  if (current !== run) return;
  let ready: Decoder;
  try {
    ready = await decoder;
  } catch {
    decoder = null;
    if (current !== run) return;
    stop();
    state.value = "nodecoder";
    return;
  }
  if (current !== run) return;
  failures = 0;
  state.value = "live";
  schedule(ready, current);
}

function schedule(ready: Decoder, current: number) {
  timer = setTimeout(() => void decodeOnce(ready, current), DECODE_EVERY_MS);
}

/** One pass; the next is scheduled only after this one ends, so passes never overlap. */
async function decodeOnce(ready: Decoder, current: number) {
  timer = null;
  if (current !== run || done) return;
  let texts: string[] = [];
  try {
    canvas ??= document.createElement("canvas");
    if (video.value && drawScanBand(video.value, canvas)) texts = await ready.detect(canvas);
    failures = 0;
  } catch {
    failures += 1;
    if (failures >= MAX_DECODE_FAILURES) {
      if (current === run) {
        stop();
        state.value = "nodecoder";
      }
      return;
    }
  }
  if (current !== run || done) return;
  for (const text of texts) {
    const isbn = isbnFromScan(text);
    if (isbn) {
      finish(isbn);
      return;
    }
  }
  schedule(ready, current);
}

function finish(isbn: string) {
  if (done) return;
  done = true;
  stop();
  emit("isbn", isbn);
}

function submitManual() {
  const isbn = parseIsbn(manual.value);
  if (!isbn) {
    manualError.value = t("errors.invalidIsbn");
    return;
  }
  manualError.value = "";
  finish(isbn);
}

function cancel() {
  done = true;
  stop();
  emit("cancel");
}

function onVisibility() {
  if (document.visibilityState === "hidden") {
    if (showVideo.value && !done) {
      resumeWhenVisible = true;
      stop();
    }
  } else if (resumeWhenVisible) {
    resumeWhenVisible = false;
    if (!done) void start();
  }
}

onMounted(() => {
  document.addEventListener("visibilitychange", onVisibility);
  void start();
});

onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", onVisibility);
  done = true;
  stop();
});
</script>

<template>
  <div class="scanner">
    <div v-if="showVideo" class="viewfinder">
      <video ref="video" playsinline muted autoplay disablepictureinpicture aria-hidden="true" />
      <div class="guide" aria-hidden="true" />
      <p v-if="state === 'starting'" class="viewfinder-note">{{ t("scan.starting") }}</p>
    </div>
    <p v-if="state === 'live'" class="fine subtle hold">{{ t("scan.hold") }}</p>

    <div v-if="!showVideo" class="problem" role="status">
      <p class="problem-text">{{ problem }}</p>
      <ul v-if="state === 'denied'" class="fine muted how-to">
        <li>{{ t("scan.deniedIphone") }}</li>
        <li>{{ t("scan.deniedAndroid") }}</li>
      </ul>
      <button v-if="canRetry" class="btn btn-ghost btn-sm" type="button" @click="start">
        {{ t("common.tryAgain") }}
      </button>
    </div>

    <form class="field manual" novalidate @submit.prevent="submitManual">
      <label class="manual-label" :for="inputId">{{ t("scan.manualLabel") }}</label>
      <div class="manual-row">
        <input
          :id="inputId"
          ref="manualInput"
          v-model="manual"
          name="isbn"
          type="text"
          maxlength="30"
          autocomplete="off"
          autocapitalize="characters"
          spellcheck="false"
          enterkeyhint="search"
          :aria-invalid="manualError ? 'true' : undefined"
          :aria-describedby="`${inputId}-hint`"
        />
        <button class="btn btn-primary" type="submit">{{ t("scan.lookUp") }}</button>
      </div>
      <span :id="`${inputId}-hint`" class="field-hint">{{ t("scan.manualHint") }}</span>
      <p v-if="manualError" class="error" role="alert">{{ manualError }}</p>
    </form>

    <div class="btn-row">
      <button class="btn btn-ghost btn-sm" type="button" @click="cancel">{{ t("common.cancel") }}</button>
    </div>
  </div>
</template>

<style scoped>
.scanner {
  display: grid;
  gap: var(--space-4);
  max-width: 560px;
}

/* The video fills a 4:3 box; the guide marks the middle 40 % that is decoded
   (or less of it, when the box crops a tall phone frame). */
.viewfinder {
  position: relative;
  overflow: hidden;
  width: 100%;
  aspect-ratio: 4 / 3;
  max-height: 55vh;
  max-height: 55dvh;
  border-radius: var(--radius-lg);
  background: #000;
}

.viewfinder video {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.guide {
  position: absolute;
  top: 30%;
  left: 8%;
  right: 8%;
  height: 40%;
  border: 2px solid rgb(255 255 255 / 0.9);
  border-radius: var(--radius-md);
  box-shadow: 0 0 0 100vmax rgb(0 0 0 / 0.35);
  pointer-events: none;
}

.guide::after {
  content: "";
  position: absolute;
  top: 50%;
  left: var(--space-3);
  right: var(--space-3);
  height: 2px;
  background: var(--accent);
  opacity: 0.85;
}

.viewfinder-note {
  position: absolute;
  inset: auto 0 var(--space-3);
  margin: 0;
  text-align: center;
  color: #fff;
  font-size: var(--text-sm);
}

.hold {
  margin: calc(-1 * var(--space-2)) 0 0;
  text-align: center;
}

.problem {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
  padding: var(--space-4);
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-lg);
  background: var(--surface-2);
}

.problem-text {
  margin: 0;
  font-weight: 600;
}

.how-to {
  margin: 0;
  padding-left: var(--space-4);
}

.manual {
  margin: 0;
}

.manual-label {
  font-size: var(--text-sm);
  font-weight: 600;
}

.manual-row {
  display: flex;
  gap: var(--space-2);
}

.manual-row input {
  flex: 1;
  min-width: 0;
}

.manual .error {
  margin: 0;
}
</style>
