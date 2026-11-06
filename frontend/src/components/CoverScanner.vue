<script setup lang="ts">
import { computed, inject, nextTick, onBeforeUnmount, ref, shallowRef, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { routerKey } from "vue-router";
import { detectCover, flatten, insetQuad, isConvex, loadForScan, MIN_CONFIDENCE, readPixels, type Point, type Quad } from "../coverScan";
import { rotateQuad } from "../coverScan/geometry";
import { ancestry, giveFocusBack } from "../focusBack";
import { useCoverScan } from "../stores/coverScan";
import { useTheme } from "../stores/theme";
import { useToast } from "../stores/toast";

/**
 * The cover scanner: a photo of a cover becomes a flat, upright picture of
 * the cover alone, the way a document-scanner app does it. The corners start
 * where `detectCover` found them (or inset from the photo's edges when it is
 * unsure), the reader drags them onto the cover's corners, and "Apply"
 * warps that quadrilateral into a rectangle (`coverScan/`). Nothing leaves
 * the browser here; the caller uploads what this resolves with.
 *
 * App.vue loads it only while `useCoverScan().request(file)` has a photo
 * open, so the detector stays out of the entry chunk. While it is open, Back
 * (or any other navigation) cancels the scan and keeps the page.
 */
type Loaded = Awaited<ReturnType<typeof loadForScan>>;
type Phase = "loading" | "ready" | "busy";

/** Long side of the copy the detector looks at. */
const DETECT_SIDE = 400;
const GLIDE_MS = 350;
const LOUPE = 110;
/** The loupe's magnification over the fitted photo. */
const LOUPE_ZOOM = 2.5;
/** Room left between the finger and the loupe. */
const LOUPE_GAP = 44;
const CORNERS = ["tl", "tr", "br", "bl"] as const;
/** An edge shorter than this on screen gets no midpoint handle: it would crowd its corners. */
const EDGE_HANDLE_MIN = 96;
/** How far outside the cover's top edge its "top" marker sits, in screen px. */
const TOP_OFFSET = 26;
const LOCKS = "sheetLocks";
const FOCUSABLE = 'button:not([disabled]), [tabindex]:not([tabindex="-1"])';

const { t } = useI18n();
const scan = useCoverScan();
const theme = useTheme();
const toast = useToast();
const keysId = useId();
// Not useRouter(): the scanner also mounts without a router (tests), and inject warns then.
const router = inject(routerKey, null);

const root = ref<HTMLElement | null>(null);
const area = ref<HTMLElement | null>(null);
const photo = ref<HTMLCanvasElement | null>(null);
const loupeCanvas = ref<HTMLCanvasElement | null>(null);

const phase = ref<Phase>("loading");
const detected = ref(false);
const loaded = shallowRef<Loaded | null>(null);
const quad = ref<Quad | null>(null);
const areaSize = ref({ w: 0, h: 0 });
/** The corner the loupe shows: the one being dragged or moved with the keys. */
const active = ref<number | null>(null);
/** A finger or mouse holds `active` (rather than the keys moving it). */
const held = ref(false);
/** Quarter turns clockwise of the result: which edge of the quad is the cover's top. */
const turns = ref(0);
/** The last "Apply" failed; said inside the dialog, where a screen reader still hears it. */
const failed = ref(false);

const file = computed(() => scan.file);
// The scanner is a photo-editing surface: dark in either mode, in the reader's palette.
const palette = computed(() => `pal-${theme.theme}-dark`);

const status = computed(() => {
  if (phase.value === "loading") return t("coverScan.loading");
  if (phase.value === "busy") return t("coverScan.applying");
  if (turns.value && topEdge.value) return t("coverScan.turned", { side: t(`coverScan.side.${topEdge.value.side}`) });
  return detected.value ? t("coverScan.found") : t("coverScan.adjust");
});

/** The photo fitted (contain) into the stage; `s` is screen px per source px. */
const fit = computed(() => {
  const image = loaded.value;
  if (!image) return null;
  const { w, h } = areaSize.value;
  const measured = w > 0 && h > 0;
  const s = measured ? Math.min(w / image.width, h / image.height) : 1;
  const dw = image.width * s;
  const dh = image.height * s;
  return { s, dw, dh, ox: measured ? (w - dw) / 2 : 0, oy: measured ? (h - dh) / 2 : 0 };
});

/** The quad in screen px, relative to the fitted photo. */
const view = computed<Point[]>(() => {
  const f = fit.value;
  if (!f || !quad.value) return [];
  return quad.value.map((p) => ({ x: p.x * f.s, y: p.y * f.s }));
});

const frameStyle = computed(() => {
  const f = fit.value;
  if (!f) return {};
  return { left: `${f.ox}px`, top: `${f.oy}px`, width: `${f.dw}px`, height: `${f.dh}px` };
});

const outline = computed(() => view.value.map((p) => `${p.x},${p.y}`).join(" "));

const shade = computed(() => {
  const f = fit.value;
  const v = view.value;
  if (!f || v.length !== 4) return "";
  const inner = v.map((p, i) => `${i ? "L" : "M"}${p.x} ${p.y}`).join(" ");
  // A pixel past the photo on every side, so no unshaded seam shows at its edge.
  return `M-1 -1H${f.dw + 1}V${f.dh + 1}H-1Z ${inner}Z`;
});

const edges = computed(() => {
  const v = view.value;
  if (v.length !== 4) return [];
  return v.map((a, i) => {
    const b = v[(i + 1) % 4];
    const length = Math.hypot(b.x - a.x, b.y - a.y);
    return {
      index: i,
      x: (a.x + b.x) / 2,
      y: (a.y + b.y) / 2,
      angle: (Math.atan2(b.y - a.y, b.x - a.x) * 180) / Math.PI,
      shown: length >= EDGE_HANDLE_MIN,
    };
  });
});

/**
 * The edge that becomes the cover's top, in screen px relative to the fitted
 * photo: its two ends, a point just outside it for the "top" marker, the angle
 * the marker reads along (upright as the cover will be), and the screen side
 * it faces. The corners themselves stay in screen order; `turns` only picks
 * which of them `rotateQuad` makes the top-left when the cover is flattened.
 */
const topEdge = computed(() => {
  const v = view.value;
  if (v.length !== 4) return null;
  const [a, b] = turns.value ? rotateQuad(v as Quad, turns.value) : (v as Quad);
  const cx = (v[0].x + v[1].x + v[2].x + v[3].x) / 4;
  const cy = (v[0].y + v[1].y + v[2].y + v[3].y) / 4;
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const length = Math.hypot(b.x - a.x, b.y - a.y) || 1;
  let nx = -(b.y - a.y) / length;
  let ny = (b.x - a.x) / length;
  // The normal that points away from the middle of the cover.
  if ((mx - cx) * nx + (my - cy) * ny < 0) {
    nx = -nx;
    ny = -ny;
  }
  const side: "top" | "right" | "bottom" | "left" =
    Math.abs(mx - cx) > Math.abs(my - cy) ? (mx > cx ? "right" : "left") : my > cy ? "bottom" : "top";
  return {
    a,
    b,
    x: mx + nx * TOP_OFFSET,
    y: my + ny * TOP_OFFSET,
    angle: (Math.atan2(b.y - a.y, b.x - a.x) * 180) / Math.PI,
    side,
  };
});

/** Where the loupe sits, in stage px: above the finger, or beside it when there is no room above. */
const loupeAt = computed(() => {
  const f = fit.value;
  const i = active.value;
  if (!f || i === null || !view.value[i]) return null;
  const { w, h } = areaSize.value;
  const px = f.ox + view.value[i].x;
  const py = f.oy + view.value[i].y;
  const half = LOUPE / 2;
  let x = px;
  let y = py - LOUPE_GAP - half;
  if (y - half < 0) {
    y = py;
    x = px < w / 2 ? px + LOUPE_GAP + half : px - LOUPE_GAP - half;
  }
  if (w > 0) x = Math.min(Math.max(x, half), w - half);
  if (h > 0) y = Math.min(Math.max(y, half), h - half);
  return { left: `${x - half}px`, top: `${y - half}px` };
});

/* ----------------------------------------------------------------- quad */

function clampPoint(p: Point): Point {
  const image = loaded.value;
  if (!image) return p;
  return { x: Math.min(Math.max(p.x, 0), image.width), y: Math.min(Math.max(p.y, 0), image.height) };
}

/** Take a candidate quad only while it stays convex; otherwise the last valid one stays. */
function propose(next: Quad): boolean {
  if (!isConvex(next)) return false;
  quad.value = next;
  return true;
}

function withPoint(base: Quad, index: number, point: Point): Quad {
  const next = base.map((p) => ({ ...p })) as Quad;
  next[index] = clampPoint(point);
  return next;
}

/* ---------------------------------------------------------------- glide */

let glideFrame = 0;
let glideTarget: Quad | null = null;

function reducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The corners glide from where they are to `target`, once, when detection lands. */
function glideTo(target: Quad) {
  const from = quad.value;
  if (!from || reducedMotion() || typeof requestAnimationFrame !== "function") {
    quad.value = target;
    return;
  }
  glideTarget = target;
  const start = performance.now();
  const step = (now: number) => {
    const k = Math.min(1, (now - start) / GLIDE_MS);
    const e = 1 - (1 - k) ** 3;
    quad.value = from.map((p, i) => ({
      x: p.x + (target[i].x - p.x) * e,
      y: p.y + (target[i].y - p.y) * e,
    })) as Quad;
    if (k < 1) glideFrame = requestAnimationFrame(step);
    else glideTarget = null;
  };
  glideFrame = requestAnimationFrame(step);
}

/** Any touch of the corners ends a glide where it was heading. */
function settleGlide() {
  if (!glideTarget) return;
  if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(glideFrame);
  quad.value = glideTarget;
  glideTarget = null;
}

/* ------------------------------------------------------------- dragging */

let drag: { pointerId: number; kind: "corner" | "edge"; index: number; x: number; y: number; origin: Quad } | null = null;

function startDrag(event: PointerEvent, kind: "corner" | "edge", index: number) {
  if (phase.value !== "ready" || !quad.value || drag) return;
  event.preventDefault();
  settleGlide();
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  drag = { pointerId: event.pointerId, kind, index, x: event.clientX, y: event.clientY, origin: quad.value };
  if (kind === "corner") {
    active.value = index;
    held.value = true;
  }
}

function moveDrag(event: PointerEvent) {
  const f = fit.value;
  if (!drag || event.pointerId !== drag.pointerId || !f) return;
  const dx = (event.clientX - drag.x) / f.s;
  const dy = (event.clientY - drag.y) / f.s;
  const { origin, index } = drag;
  if (drag.kind === "corner") {
    propose(withPoint(origin, index, { x: origin[index].x + dx, y: origin[index].y + dy }));
    return;
  }
  // An edge moves both of its corners by the same amount, as far as both stay on the photo.
  const image = loaded.value;
  if (!image) return;
  const a = origin[index];
  const b = origin[(index + 1) % 4];
  const mx = Math.min(Math.max(dx, -Math.min(a.x, b.x)), image.width - Math.max(a.x, b.x));
  const my = Math.min(Math.max(dy, -Math.min(a.y, b.y)), image.height - Math.max(a.y, b.y));
  const next = withPoint(origin, index, { x: a.x + mx, y: a.y + my });
  propose(withPoint(next, (index + 1) % 4, { x: b.x + mx, y: b.y + my }));
}

function endDrag(event: PointerEvent) {
  if (!drag || event.pointerId !== drag.pointerId) return;
  drag = null;
  active.value = null;
  held.value = false;
}

function onCornerKey(event: KeyboardEvent, index: number) {
  const moves: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
  };
  const move = moves[event.key];
  if (!move || !quad.value || phase.value !== "ready") return;
  event.preventDefault();
  settleGlide();
  // One screen pixel a press, ten with Shift.
  const step = (event.shiftKey ? 10 : 1) / (fit.value?.s || 1);
  const p = quad.value[index];
  propose(withPoint(quad.value, index, { x: p.x + move[0] * step, y: p.y + move[1] * step }));
  active.value = index;
}

function onCornerBlur(index: number) {
  if (!drag && active.value === index) active.value = null;
}

/* -------------------------------------------------------------- drawing */

function drawPhoto() {
  const canvas = photo.value;
  const f = fit.value;
  const image = loaded.value;
  if (!canvas || !f || !image) return;
  const dpr = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.min(4096, Math.round(f.dw * dpr)));
  const height = Math.max(1, Math.min(4096, Math.round(f.dh * dpr)));
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(image.bitmap, 0, 0, image.width, image.height, 0, 0, width, height);
}

/** The loupe: the photo around the active corner, zoomed, with the corner's two edges. */
function drawLoupe() {
  const canvas = loupeCanvas.value;
  const f = fit.value;
  const image = loaded.value;
  const i = active.value;
  if (!canvas || !f || !image || i === null || !quad.value) return;
  const dpr = window.devicePixelRatio || 1;
  const size = Math.round(LOUPE * dpr);
  if (canvas.width !== size) canvas.width = size;
  if (canvas.height !== size) canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const zoom = f.s * LOUPE_ZOOM;
  const half = LOUPE / 2 / zoom;
  const p = quad.value[i];
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, LOUPE, LOUPE);
  // Only the part of the photo that exists; past its edge the loupe stays dark.
  const sx0 = Math.max(0, p.x - half);
  const sy0 = Math.max(0, p.y - half);
  const sx1 = Math.min(image.width, p.x + half);
  const sy1 = Math.min(image.height, p.y + half);
  if (sx1 > sx0 && sy1 > sy0) {
    ctx.drawImage(
      image.bitmap,
      sx0,
      sy0,
      sx1 - sx0,
      sy1 - sy0,
      (sx0 - (p.x - half)) * zoom,
      (sy0 - (p.y - half)) * zoom,
      (sx1 - sx0) * zoom,
      (sy1 - sy0) * zoom,
    );
  }
  const accent = root.value ? getComputedStyle(root.value).getPropertyValue("--accent").trim() : "";
  if (!accent) return;
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2;
  ctx.beginPath();
  for (const j of [(i + 3) % 4, (i + 1) % 4]) {
    const q = quad.value[j];
    ctx.moveTo(LOUPE / 2, LOUPE / 2);
    ctx.lineTo(LOUPE / 2 + (q.x - p.x) * zoom, LOUPE / 2 + (q.y - p.y) * zoom);
  }
  ctx.stroke();
}

watch([fit, photo], drawPhoto, { flush: "post" });
watch([quad, active, loupeCanvas], drawLoupe, { flush: "post" });

/* ------------------------------------------------------------- open/close */

let session = 0;
/** Between open() and close(): the listeners, the scroll lock and the focus to give back. */
let opened = false;
let observer: ResizeObserver | null = null;
/** The element to give focus back to, then its ancestors, nearest first. */
let restoreFocusTo: HTMLElement[] = [];
let removeGuard: (() => void) | null = null;
/** The browser chrome's colour before the scanner darkened it; null while untouched. */
let savedThemeColor: string | null = null;

function measure() {
  const el = area.value;
  if (!el) return;
  areaSize.value = { w: el.clientWidth, h: el.clientHeight };
}

function afterPaint(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => setTimeout(resolve, 0));
    else setTimeout(resolve, 0);
  });
}

function onKeydown(event: KeyboardEvent) {
  // On the window, in the capture phase: ahead of any sheet still listening underneath.
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopImmediatePropagation();
    if (phase.value !== "busy") cancel();
    return;
  }
  if (event.key !== "Tab" || !root.value) return;
  const items = Array.from(root.value.querySelectorAll<HTMLElement>(FOCUSABLE));
  if (items.length === 0) return;
  const first = items[0];
  const last = items[items.length - 1];
  const current = document.activeElement as HTMLElement | null;
  // The dialog itself (focused when it opens) counts as outside: from there the
  // browser would tab backwards out of it, onto the page hidden underneath.
  const outside = !current || current === root.value || !root.value.contains(current);
  if (event.shiftKey && (current === first || outside)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (current === last || outside)) {
    event.preventDefault();
    first.focus();
  }
}

function lock() {
  document.body.dataset[LOCKS] = String(Number(document.body.dataset[LOCKS] ?? 0) + 1);
  document.body.style.overflow = "hidden";
}

function unlock() {
  const locks = Math.max(0, Number(document.body.dataset[LOCKS] ?? 1) - 1);
  if (locks) document.body.dataset[LOCKS] = String(locks);
  else {
    delete document.body.dataset[LOCKS];
    document.body.style.overflow = "";
  }
}

/** The browser chrome (Android's status bar, Safari's tinted bar) takes the scanner's dark background. */
function darkenChrome() {
  const meta = document.querySelector('meta[name="theme-color"]');
  if (!meta || !root.value) return;
  const bg = getComputedStyle(root.value).getPropertyValue("--bg").trim();
  if (!bg) return;
  if (savedThemeColor === null) savedThemeColor = meta.getAttribute("content") ?? "";
  meta.setAttribute("content", bg);
}

function restoreChrome() {
  if (savedThemeColor === null) return;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", savedThemeColor);
  savedThemeColor = null;
}

function release(image: Loaded | null) {
  const bitmap = image?.bitmap as { close?: () => void } | undefined;
  bitmap?.close?.();
}

/** The detector's corners, in source px, or null when it is unsure. */
function detect(image: Loaded): Quad | null {
  try {
    const { data } = readPixels(image.bitmap, image.width, image.height, DETECT_SIDE);
    const hit = detectCover(data);
    if (!hit || hit.confidence < MIN_CONFIDENCE || !data.width) return null;
    const k = image.width / data.width;
    const found = hit.quad.map((p) => clampPoint({ x: p.x * k, y: p.y * k })) as Quad;
    return isConvex(found) ? found : null;
  } catch {
    return null;
  }
}

async function open(chosen: File) {
  const run = ++session;
  opened = true;
  phase.value = "loading";
  detected.value = false;
  failed.value = false;
  turns.value = 0;
  // The store notes the focus when the photo was requested: by the time this
  // (lazily loaded) screen opens, the opener may already be disabled and blurred.
  restoreFocusTo = ancestry(scan.opener ?? document.activeElement);
  window.addEventListener("keydown", onKeydown, true);
  // Back, or any other navigation, cancels the scan and keeps the page (on
  // popstate vue-router puts the URL back): the photo belongs to this page.
  removeGuard?.();
  removeGuard =
    router?.beforeEach(() => {
      if (!file.value) return true;
      scan.finish(null);
      return false;
    }) ?? null;
  lock();
  await nextTick();
  if (run !== session) return;
  root.value?.focus({ preventScroll: true });
  darkenChrome();
  if (area.value && typeof ResizeObserver === "function") {
    observer = new ResizeObserver(measure);
    observer.observe(area.value);
  }
  measure();

  let image: Loaded;
  try {
    image = await loadForScan(chosen);
  } catch {
    // A photo this browser cannot open goes up as it is; the server decides.
    if (run === session) scan.finish(chosen);
    return;
  }
  if (run !== session) {
    release(image);
    return;
  }
  loaded.value = image;
  quad.value = insetQuad(image.width, image.height);
  measure();
  // The photo and the inset corners paint first; then the detector runs.
  await afterPaint();
  if (run !== session) return;
  const found = detect(image);
  detected.value = found !== null;
  phase.value = "ready";
  if (found) glideTo(found);
}

function close() {
  session += 1;
  if (!opened) return;
  opened = false;
  if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(glideFrame);
  glideTarget = null;
  drag = null;
  active.value = null;
  held.value = false;
  observer?.disconnect();
  observer = null;
  window.removeEventListener("keydown", onKeydown, true);
  removeGuard?.();
  removeGuard = null;
  release(loaded.value);
  loaded.value = null;
  quad.value = null;
  phase.value = "loading";
  failed.value = false;
  turns.value = 0;
  unlock();
  restoreChrome();
  const chain = restoreFocusTo;
  restoreFocusTo = [];
  // A CoverPhotoButton disables its file input while it uploads: then its label or an ancestor.
  giveFocusBack(chain);
}

// Immediate: App.vue mounts this screen only once a photo is already waiting.
watch(
  file,
  (next, previous) => {
    if (previous) close();
    if (next) void open(next);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (file.value) scan.finish(null);
  close();
});

/* -------------------------------------------------------------- actions */

// While busy the buttons are aria-disabled rather than disabled: a disabled
// button drops the focus it holds to the body.
function cancel() {
  if (phase.value === "busy") return;
  scan.finish(null);
}

function whole() {
  const chosen = file.value;
  if (phase.value === "busy" || !chosen) return;
  scan.finish(chosen);
}

/** The cover's top moves one edge on, clockwise: for a cover photographed sideways. */
function rotate() {
  if (phase.value !== "ready") return;
  settleGlide();
  turns.value = (turns.value + 1) % 4;
}

async function apply() {
  const image = loaded.value;
  if (phase.value !== "ready" || !image || !quad.value) return;
  settleGlide();
  const run = session;
  phase.value = "busy";
  failed.value = false;
  // The warp blocks the main thread for a while on a phone: "Straightening…" paints first.
  await nextTick();
  await afterPaint();
  if (run !== session || !quad.value) return;
  const corners = turns.value ? rotateQuad(quad.value, turns.value) : quad.value;
  try {
    const flat = await flatten(image.bitmap, image.width, image.height, corners);
    if (run === session) scan.finish(flat);
  } catch {
    if (run !== session) return;
    phase.value = "ready";
    failed.value = true;
    toast.show(t("coverScan.failed"));
  }
}
</script>

<template>
  <div
    v-if="file"
    ref="root"
    class="scanner"
    :class="palette"
    role="dialog"
    aria-modal="true"
    :aria-label="t('coverScan.title')"
    :aria-busy="phase !== 'ready'"
    tabindex="-1"
  >
    <p class="scan-status" aria-live="polite">{{ status }}</p>

    <div class="scan-stage">
      <div ref="area" class="scan-area">
        <div v-if="fit && quad" class="scan-frame" :class="{ ready: phase !== 'loading' }" :style="frameStyle">
          <canvas ref="photo" class="scan-photo" aria-hidden="true" />
          <svg
            class="scan-overlay"
            :viewBox="`0 0 ${fit.dw} ${fit.dh}`"
            :width="fit.dw"
            :height="fit.dh"
            aria-hidden="true"
          >
            <path class="scan-shade" :d="shade" fill-rule="evenodd" />
            <polygon class="scan-edge-under" :points="outline" />
            <polygon class="scan-edge-line" :points="outline" pathLength="1" />
            <line
              v-if="topEdge && phase !== 'loading'"
              class="scan-edge-top"
              :x1="topEdge.a.x"
              :y1="topEdge.a.y"
              :x2="topEdge.b.x"
              :y2="topEdge.b.y"
            />
          </svg>
          <span
            v-if="topEdge && phase !== 'loading'"
            class="scan-top"
            :data-side="topEdge.side"
            :style="{ left: `${topEdge.x}px`, top: `${topEdge.y}px`, '--angle': `${topEdge.angle}deg` }"
            aria-hidden="true"
            >{{ t("coverScan.top") }}</span
          >
          <template v-if="phase !== 'loading'">
            <span
              v-for="edge in edges"
              v-show="edge.shown"
              :key="`edge-${edge.index}`"
              class="scan-handle scan-edge"
              :style="{ left: `${edge.x}px`, top: `${edge.y}px`, '--angle': `${edge.angle}deg` }"
              aria-hidden="true"
              @pointerdown="startDrag($event, 'edge', edge.index)"
              @pointermove="moveDrag"
              @pointerup="endDrag"
              @pointercancel="endDrag"
            />
            <button
              v-for="(corner, i) in CORNERS"
              :key="corner"
              type="button"
              class="scan-handle scan-corner"
              :class="{ held: held && active === i }"
              :data-corner="corner"
              :style="{ left: `${view[i]?.x ?? 0}px`, top: `${view[i]?.y ?? 0}px` }"
              :aria-label="t(`coverScan.corner.${corner}`)"
              :aria-describedby="keysId"
              @pointerdown="startDrag($event, 'corner', i)"
              @pointermove="moveDrag"
              @pointerup="endDrag"
              @pointercancel="endDrag"
              @keydown="onCornerKey($event, i)"
              @blur="onCornerBlur(i)"
            />
          </template>
        </div>
        <div v-if="loupeAt" class="scan-loupe" :style="loupeAt" aria-hidden="true">
          <canvas ref="loupeCanvas" />
        </div>
      </div>
    </div>

    <p v-if="failed" class="scan-error" role="alert">{{ t("coverScan.failed") }}</p>

    <div class="scan-bar">
      <button type="button" class="text-btn scan-cancel" :aria-disabled="phase === 'busy'" @click="cancel">
        {{ t("common.cancel") }}
      </button>
      <button
        type="button"
        class="icon-btn scan-rotate"
        :aria-label="t('coverScan.rotate')"
        :title="t('coverScan.rotate')"
        :aria-disabled="phase !== 'ready'"
        @click="rotate"
      >
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M20 11a8 8 0 1 0-2.34 5.66" />
          <path d="M20 4v7h-7" />
        </svg>
      </button>
      <span class="scan-bar-end">
        <button type="button" class="btn btn-ghost scan-whole" :aria-disabled="phase === 'busy'" @click="whole">
          {{ t("coverScan.whole") }}
        </button>
        <button type="button" class="btn btn-primary scan-apply" :aria-disabled="phase !== 'ready'" @click="apply">
          {{ phase === "busy" ? t("coverScan.applying") : t("coverScan.apply") }}
        </button>
      </span>
    </div>
    <p :id="keysId" hidden>{{ t("coverScan.keys") }}</p>
  </div>
</template>

<style scoped>
/* Above sheets (40/41), below toasts (60), so a failure still shows. */
.scanner {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  animation: scan-in var(--dur-base) var(--ease-out);
}

.scanner:focus-visible {
  outline: none;
}

.scan-status {
  margin: 0;
  padding: calc(var(--space-3) + env(safe-area-inset-top)) var(--space-4) var(--space-2);
  text-align: center;
  font-size: var(--text-sm);
  color: var(--text-muted);
  min-height: calc(var(--text-sm) * 1.55 + var(--space-5));
}

/* The stage takes the finger: no scroll, no pinch, no text selection. */
.scan-stage {
  position: relative;
  flex: 1;
  min-height: 0;
  touch-action: none;
  user-select: none;
  -webkit-user-select: none;
  -webkit-touch-callout: none;
}

/* Inset, so a corner at the photo's edge keeps its whole hit area on screen. */
.scan-area {
  position: absolute;
  inset: var(--space-6);
}

.scan-frame {
  position: absolute;
}

.scan-photo,
.scan-overlay {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  display: block;
}

.scan-overlay {
  overflow: visible;
  pointer-events: none;
}

.scan-shade {
  fill: var(--overlay);
}

.scan-edge-under,
.scan-edge-line {
  fill: none;
  stroke-linejoin: round;
}

.scan-edge-under {
  stroke: var(--bg);
  stroke-opacity: 0.5;
  stroke-width: 4;
}

.scan-edge-line {
  stroke: var(--accent);
  stroke-width: 2;
}

/* The edge that becomes the cover's top: drawn heavier, with a small "top" beside it. */
.scan-edge-top {
  stroke: var(--accent);
  stroke-width: 5;
  stroke-linecap: round;
}

.scan-top {
  position: absolute;
  transform: translate(-50%, -50%) rotate(var(--angle, 0deg));
  padding: 1px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--accent);
  color: var(--accent-ink);
  font-size: var(--text-xs);
  font-weight: 600;
  line-height: 1.4;
  white-space: nowrap;
  pointer-events: none;
}

.scan-frame:not(.ready) .scan-top {
  visibility: hidden;
}

/* The shade and the edges wait for the detector, then the edges draw in once. */
.scan-frame:not(.ready) .scan-overlay {
  visibility: hidden;
}

.scan-frame.ready .scan-shade {
  animation: scan-in var(--dur-slow) var(--ease-out);
}

.scan-frame.ready .scan-edge-line {
  stroke-dasharray: 1;
  stroke-dashoffset: 0;
  animation: edges-in var(--dur-slow) var(--ease-out);
}

/* A handle is a 48 px hit area around a smaller mark. */
.scan-handle {
  position: absolute;
  width: 48px;
  height: 48px;
  margin: -24px 0 0 -24px;
  padding: 0;
  border: 0;
  background: transparent;
  touch-action: none;
  cursor: grab;
}

.scan-handle:active {
  cursor: grabbing;
}

.scan-handle::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
}

.scan-corner::after {
  width: 24px;
  height: 24px;
  margin: -12px 0 0 -12px;
  border-radius: 50%;
  border: 3px solid var(--accent);
  box-shadow:
    0 0 0 1px var(--bg),
    inset 0 0 0 1px var(--bg);
  transition:
    transform var(--dur-fast) var(--ease-out),
    opacity var(--dur-fast) var(--ease);
}

/* While held, the loupe shows the corner; the mark under the finger shrinks away. */
.scan-corner.held::after {
  transform: scale(0.6);
  opacity: 0.5;
}

.scan-corner:focus-visible {
  outline: none;
}

.scan-corner:focus-visible::after {
  box-shadow:
    0 0 0 2px var(--bg),
    0 0 0 4px var(--text);
}

.scan-edge {
  width: 40px;
  height: 40px;
  margin: -20px 0 0 -20px;
}

.scan-edge::after {
  width: 22px;
  height: 6px;
  margin: -3px 0 0 -11px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  box-shadow: 0 0 0 1px var(--bg);
  transform: rotate(var(--angle, 0deg));
}

.scan-loupe {
  position: absolute;
  width: 110px;
  height: 110px;
  border-radius: 50%;
  overflow: hidden;
  background: var(--surface);
  border: 2px solid var(--accent);
  box-shadow: var(--shadow-3);
  pointer-events: none;
}

.scan-loupe canvas {
  display: block;
  width: 100%;
  height: 100%;
}

/* The crosshair marks the exact point the corner sits on. */
.scan-loupe::before,
.scan-loupe::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  background: var(--text);
}

.scan-loupe::before {
  width: 18px;
  height: 1px;
  margin: -0.5px 0 0 -9px;
}

.scan-loupe::after {
  width: 1px;
  height: 18px;
  margin: -9px 0 0 -0.5px;
}

.scan-error {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  text-align: center;
  font-size: var(--text-sm);
  color: var(--danger);
  background: var(--surface);
}

.scan-bar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-3) calc(var(--space-3) + env(safe-area-inset-bottom));
  background: var(--surface);
  border-top: 1px solid var(--border);
}

/* Cancel at the left, the two choices share the rest; within thumb reach. */
.scan-bar-end {
  display: flex;
  flex: 1;
  justify-content: flex-end;
  gap: var(--space-2);
  min-width: 0;
}

.scan-bar .btn {
  flex: 1 1 0;
  max-width: 180px;
  min-width: 0;
  padding: 0 var(--space-3);
  line-height: var(--leading-tight);
  text-align: center;
}

.scan-bar .btn,
.scan-bar .text-btn {
  white-space: nowrap;
}

/* The primary choice takes a little more: room for its busy label, with no jump when it shows. */
.scan-bar .scan-apply {
  flex-grow: 1.3;
}

.scan-bar [aria-disabled="true"] {
  opacity: 0.55;
  cursor: default;
}

.scan-cancel,
.scan-rotate {
  flex: none;
}

.scan-cancel {
  padding: 0 var(--space-2);
}

/* At 360 px the four controls and the longest label ("Wird begradigt …") fit on one line. */
@media (max-width: 400px) {
  .scan-bar {
    padding-inline: var(--space-2);
  }

  .scan-bar,
  .scan-bar-end {
    gap: var(--space-1);
  }

  .scan-bar .btn,
  .scan-bar .text-btn {
    font-size: var(--text-sm);
  }

  .scan-bar .btn {
    padding: 0 var(--space-1);
  }
}

@media (min-width: 720px) {
  .scan-bar {
    justify-content: center;
    padding-inline: var(--space-6);
  }

  .scan-bar-end {
    flex: none;
  }

  .scan-bar .btn {
    flex: none;
    max-width: none;
    padding: 0 var(--space-5);
  }
}

@keyframes scan-in {
  from {
    opacity: 0;
  }
}

@keyframes edges-in {
  from {
    stroke-dashoffset: 1;
  }
}

@media (prefers-reduced-motion: reduce) {
  .scan-frame.ready .scan-edge-line {
    animation: none;
  }
}
</style>
