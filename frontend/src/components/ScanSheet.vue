<script setup lang="ts">
/**
 * ISBN entry: camera barcode scan where BarcodeDetector exists (Chrome/Android,
 * Safari 17+), manual ISBN typing everywhere. Both resolve through the same
 * lookup and land in the book detail sheet.
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import { useFlow } from "../stores/flow";
import NavIcon from "./NavIcon.vue";
import Sheet from "./Sheet.vue";

type Detector = { detect(source: ImageBitmapSource): Promise<{ rawValue: string; format: string }[]> };
type DetectorCtor = new (options?: { formats?: string[] }) => Detector;

const emit = defineEmits<{
  close: [];
}>();

const flow = useFlow();
const video = ref<HTMLVideoElement | null>(null);
const supported = ref(false);
const scanning = ref(false);
const cameraError = ref("");
const manual = ref("");
const lookupError = ref("");
const pending = ref(false);
let stream: MediaStream | null = null;
let detector: Detector | null = null;
let timer = 0;
let done = false;

function detectorCtor(): DetectorCtor | null {
  const ctor = (window as unknown as { BarcodeDetector?: DetectorCtor }).BarcodeDetector;
  return ctor ?? null;
}

async function startCamera() {
  const Ctor = detectorCtor();
  if (!Ctor || !navigator.mediaDevices?.getUserMedia) {
    supported.value = false;
    return;
  }
  supported.value = true;
  try {
    detector = new Ctor({ formats: ["ean_13", "ean_8", "upc_a", "isbn"] });
    stream = await navigator.mediaDevices.getUserMedia({
      video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 } },
      audio: false,
    });
    if (video.value) {
      video.value.srcObject = stream;
      await video.value.play();
    }
    scanning.value = true;
    tick();
  } catch {
    cameraError.value = "Camera not available — type the ISBN instead.";
    scanning.value = false;
  }
}

async function tick() {
  if (done || !detector || !video.value || video.value.readyState < 2) {
    timer = window.setTimeout(tick, 250);
    return;
  }
  try {
    const codes = await detector.detect(video.value);
    const hit = codes.find((code) => /^\d{9}[\dX]$|^\d{13}$/.test(code.rawValue.replace(/[-\s]/g, "")));
    if (hit) {
      await resolve(hit.rawValue);
      return;
    }
  } catch {
    /* keep scanning */
  }
  timer = window.setTimeout(tick, 220);
}

async function resolve(isbn: string) {
  if (pending.value || done) return;
  pending.value = true;
  lookupError.value = "";
  try {
    const hit = await api.isbnLookup(isbn);
    done = true;
    stopCamera();
    if (navigator.vibrate) navigator.vibrate(30);
    flow.open({
      kind: "details",
      book: {
        ol_work_key: hit.ol_work_key,
        title: hit.title,
        authors: hit.authors,
        cover_id: hit.cover_id,
        year: hit.year,
      },
    });
  } catch (err) {
    lookupError.value = err instanceof ApiError ? err.message : "Could not look that up";
  } finally {
    pending.value = false;
  }
}

function stopCamera() {
  window.clearTimeout(timer);
  stream?.getTracks().forEach((track) => track.stop());
  stream = null;
  scanning.value = false;
}

onMounted(startCamera);
onBeforeUnmount(stopCamera);
</script>

<template>
  <Sheet title="Scan a book" subtitle="Point the camera at the barcode on the back" @close="emit('close')">
    <div v-if="supported && !cameraError" class="scanner" style="margin-bottom: 14px">
      <video ref="video" playsinline muted />
      <div class="reticle" aria-hidden="true" />
    </div>
    <p v-else-if="cameraError" class="muted fine" style="margin-bottom: 12px">{{ cameraError }}</p>
    <p v-else class="muted fine" style="margin-bottom: 12px">
      This browser can’t scan barcodes. Type the ISBN from the back cover.
    </p>
    <form @submit.prevent="resolve(manual)">
      <label class="field">
        <span>ISBN</span>
        <input
          v-model="manual"
          inputmode="numeric"
          autocomplete="off"
          placeholder="978…"
          :data-autofocus="!supported ? true : undefined"
        />
        <span v-if="lookupError" class="error fine">{{ lookupError }}</span>
      </label>
      <button class="btn btn-primary btn-block" type="submit" :disabled="pending || manual.trim().length < 10">
        <NavIcon v-if="!pending" name="search" :size="18" />
        {{ pending ? "Looking up…" : "Look up" }}
      </button>
    </form>
  </Sheet>
</template>
