<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  title: string;
  /**
   * The sheet holds a change made without typing (a stepper, a slider moved
   * by script): a drag springs back as after an edit. Typing and choosing set
   * that on their own (`input`, `change`).
   */
  dirty?: boolean;
}>();

const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();

const backdrop = ref<HTMLElement | null>(null);
const panel = ref<HTMLElement | null>(null);
let restoreFocusTo: HTMLElement | null = null;

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusables(): HTMLElement[] {
  if (!panel.value) return [];
  return Array.from(panel.value.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) => element.offsetParent !== null,
  );
}

function matches(query: string): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia(query).matches;
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    emit("close");
    return;
  }
  if (event.key !== "Tab") return;
  const items = focusables();
  if (items.length === 0) return;
  const first = items[0];
  const last = items[items.length - 1];
  const active = document.activeElement as HTMLElement | null;
  // Cycle within the sheet so Tab can never reach the page behind it.
  if (event.shiftKey && (active === first || !panel.value?.contains(active))) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

/* ------------------------------------------------------- drag to dismiss */

// The sheet follows the finger down; let go past a third of its height (or
// with a flick) and it closes, otherwise it springs back. A drag starts on
// the grip and the title row, or in the body while it is scrolled to the top.
let dragStartY = 0;
let dragLastY = 0;
let dragLastT = 0;
let velocity = 0;
let dragging = false;
let fromBody = false;
let offset = 0;
// Once something was typed or chosen in the sheet, a drag no longer closes
// it: a swipe made in passing would throw the draft away. The sheet gives a
// little and springs back; the close button is the deliberate way out.
let edited = false;

function onEdit() {
  edited = true;
}

/** A drag only gives a little and springs back: something in the sheet would be lost. */
function holding(): boolean {
  return edited || Boolean(props.dirty);
}

function setOffset(y: number) {
  offset = Math.max(0, y);
  if (!panel.value || !backdrop.value) return;
  panel.value.style.transform = offset ? `translateY(${offset}px)` : "";
  const height = panel.value.offsetHeight || 1;
  backdrop.value.style.setProperty("--sheet-shade", String(Math.max(0, 1 - offset / height)));
}

function begin(y: number, body: boolean) {
  dragStartY = y;
  dragLastY = y;
  dragLastT = performance.now();
  velocity = 0;
  dragging = !body;
  fromBody = body;
  panel.value?.classList.remove("settling");
}

function move(y: number): boolean {
  const now = performance.now();
  const dy = y - dragStartY;
  if (!dragging) {
    // In the body a drag only starts downward from the top; anything else
    // is the body's own scroll.
    if (!fromBody || holding() || dy < 8 || (panel.value?.scrollTop ?? 0) > 0) return false;
    dragging = true;
    dragStartY = y;
  }
  velocity = (y - dragLastY) / Math.max(1, now - dragLastT);
  dragLastY = y;
  dragLastT = now;
  const pulled = y - dragStartY;
  setOffset(holding() ? Math.min(48, pulled * 0.25) : pulled);
  return true;
}

async function end() {
  if (!dragging) return;
  dragging = false;
  const height = panel.value?.offsetHeight ?? 0;
  // A flick counts only if the finger was still moving when it let go.
  const flick = velocity > 0.6 && offset > 40 && performance.now() - dragLastT < 100;
  if (!holding() && (offset > Math.min(160, height / 3) || flick)) {
    emit("close");
    // A parent may refuse to close (it is still saving); then come back up.
    await nextTick();
    if (panel.value) settle();
    return;
  }
  settle();
}

function settle() {
  panel.value?.classList.add("settling");
  setOffset(0);
}

function onGripDown(event: PointerEvent) {
  if ((event.target as HTMLElement).closest("button")) return;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  begin(event.clientY, false);
}

function onGripMove(event: PointerEvent) {
  if (dragging && !fromBody) move(event.clientY);
}

function onTouchStart(event: TouchEvent) {
  // A range input's thumb moves sideways under the finger; a slightly
  // downward drag on it must not pull the sheet away with the draft.
  const target = event.target as HTMLElement;
  if (event.touches.length !== 1 || target.closest(".sheet-grip, input[type='range']")) return;
  begin(event.touches[0].clientY, true);
}

function onTouchMove(event: TouchEvent) {
  if (!fromBody || event.touches.length !== 1) return;
  if (move(event.touches[0].clientY) && event.cancelable) event.preventDefault();
}

function onTouchEnd() {
  if (fromBody) void end();
  fromBody = false;
}

/* ------------------------------------------------------------ open/close */

// A count, so a sheet opened from a sheet does not unlock the page when the
// inner one closes.
const LOCKS = "sheetLocks";

onMounted(() => {
  restoreFocusTo = document.activeElement as HTMLElement | null;
  document.addEventListener("keydown", onKeydown, true);
  document.body.dataset[LOCKS] = String(Number(document.body.dataset[LOCKS] ?? 0) + 1);
  document.body.style.overflow = "hidden";
  panel.value?.addEventListener("touchmove", onTouchMove, { passive: false });
  // With a mouse, the first field is where the hand already is. On a phone,
  // focusing a field would throw the keyboard over the sheet before it has
  // even arrived, so the sheet itself takes focus (and its label is read).
  const first = focusables().find((element) => !element.classList.contains("sheet-close"));
  (matches("(pointer: fine)") ? (first ?? panel.value) : panel.value)?.focus({ preventScroll: true });
});

// The parent removes the sheet with v-if, so there is no leave transition to
// hook. Instead a copy of the sheet as it stands slides away above the page
// while the real one is already gone.
onBeforeUnmount(() => {
  const root = backdrop.value;
  if (!root || typeof root.animate !== "function" || matches("(prefers-reduced-motion: reduce)")) return;
  const ghost = root.cloneNode(true) as HTMLElement;
  const fields = root.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea");
  ghost.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea").forEach((field, index) => {
    field.value = fields[index]?.value ?? field.value;
  });
  ghost.setAttribute("aria-hidden", "true");
  ghost.setAttribute("inert", "");
  ghost.classList.add("leaving");
  document.body.appendChild(ghost);
  const sheet = ghost.querySelector<HTMLElement>(".sheet");
  if (sheet) sheet.scrollTop = panel.value?.scrollTop ?? 0;
  const desktop = matches("(min-width: 720px)");
  const duration = desktop ? 140 : 240;
  // Only the shade fades; the sheet itself stays opaque while it slides.
  ghost.animate([{ opacity: Number(root.style.getPropertyValue("--sheet-shade") || 1) }, { opacity: 0 }], {
    duration,
    easing: "ease-in",
    fill: "forwards",
    pseudoElement: "::before",
  });
  const away = sheet?.animate(
    desktop
      ? [{ opacity: 1, transform: "scale(1)" }, { opacity: 0, transform: "scale(0.97)" }]
      : [{ transform: `translateY(${offset}px)` }, { transform: "translateY(100%)" }],
    { duration, easing: "cubic-bezier(0.5, 0, 0.75, 0)", fill: "forwards" },
  );
  if (!away) {
    ghost.remove();
    return;
  }
  away.onfinish = () => ghost.remove();
  away.oncancel = () => ghost.remove();
});

onUnmounted(() => {
  document.removeEventListener("keydown", onKeydown, true);
  const locks = Math.max(0, Number(document.body.dataset[LOCKS] ?? 1) - 1);
  if (locks) document.body.dataset[LOCKS] = String(locks);
  else {
    delete document.body.dataset[LOCKS];
    document.body.style.overflow = "";
  }
  restoreFocusTo?.focus?.({ preventScroll: true });
});
</script>

<template>
  <div ref="backdrop" class="sheet-backdrop" @click.self="emit('close')">
    <div
      ref="panel"
      class="sheet"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
      tabindex="-1"
      @input="onEdit"
      @change="onEdit"
      @touchstart.passive="onTouchStart"
      @touchend="onTouchEnd"
      @touchcancel="onTouchEnd"
    >
      <div
        class="sheet-grip"
        @pointerdown="onGripDown"
        @pointermove="onGripMove"
        @pointerup="end"
        @pointercancel="end"
      >
        <span class="sheet-handle" aria-hidden="true" />
        <div class="sheet-head">
          <h2>{{ title }}</h2>
          <button class="icon-btn sheet-close" type="button" :aria-label="t('common.close')" @click="emit('close')">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
              <path d="M6 6l12 12M18 6 6 18" />
            </svg>
          </button>
        </div>
      </div>
      <slot />
    </div>
  </div>
</template>

<style scoped>
.sheet-backdrop {
  --sheet-shade: 1;
  position: fixed;
  inset: 0;
  z-index: 40;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

/* The shade is its own layer so a drag can thin it without touching the sheet. */
.sheet-backdrop::before {
  content: "";
  position: absolute;
  inset: 0;
  background: var(--overlay);
  opacity: var(--sheet-shade);
  animation: shade-in var(--dur-slow) var(--ease-out);
}

.sheet-backdrop.leaving {
  z-index: 41;
  pointer-events: none;
}

.sheet {
  position: relative;
  width: min(100%, 480px);
  max-height: 92dvh;
  overflow-y: auto;
  overscroll-behavior: contain;
  background: var(--bg);
  border: 1px solid var(--border);
  border-bottom: 0;
  border-radius: var(--radius-xl) var(--radius-xl) 0 0;
  padding: 0 var(--space-4) calc(var(--space-5) + env(safe-area-inset-bottom));
  box-shadow: var(--elev-float);
  animation: sheet-in var(--dur-slow) var(--ease-out);
}

.sheet:focus-visible {
  outline: none;
}

/* Springing back after a short drag. */
.sheet.settling {
  transition: transform var(--dur-base) var(--ease-out);
}

/* The grip stays at the top while the body scrolls under it. */
.sheet-grip {
  position: sticky;
  top: 0;
  z-index: 2;
  margin: 0 calc(-1 * var(--space-4)) var(--space-3);
  padding: var(--space-2) var(--space-4) 0;
  background: var(--bg);
  touch-action: none;
  cursor: grab;
}

.sheet-handle {
  display: block;
  width: 36px;
  height: 4px;
  margin: 0 auto var(--space-1);
  border-radius: var(--radius-pill);
  background: var(--border-strong);
}

.sheet-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.sheet-head h2 {
  min-width: 0;
  font-size: var(--text-xl);
  overflow-wrap: anywhere;
}

.sheet-close {
  flex: none;
  margin-right: calc(-1 * var(--space-2));
}

@keyframes shade-in {
  from {
    opacity: 0;
  }
}

@keyframes sheet-in {
  from {
    transform: translateY(100%);
  }
}

@keyframes sheet-pop {
  from {
    opacity: 0;
    transform: scale(0.97);
  }
}

@media (min-width: 720px) {
  .sheet-backdrop {
    align-items: center;
  }

  .sheet {
    border-bottom: 1px solid var(--border);
    border-radius: var(--radius-xl);
    padding-bottom: var(--space-5);
    animation: sheet-pop var(--dur-base) var(--ease-out);
  }

  .sheet-handle {
    visibility: hidden;
    height: var(--space-2);
  }

  .sheet-grip {
    touch-action: auto;
    cursor: auto;
  }
}
</style>
