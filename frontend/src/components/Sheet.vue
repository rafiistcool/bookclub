<script setup lang="ts">
/**
 * The one bottom sheet. Drag handle + swipe down to close, Escape, backdrop
 * tap, focus trap, body scroll lock, internal scroll for tall content.
 *
 * Slots: default (body), `foot` (fixed action row), `head-actions`.
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import NavIcon from "./NavIcon.vue";

const props = defineProps<{
  title: string;
  subtitle?: string;
  closeLabel?: string;
  hideClose?: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const root = ref<HTMLElement | null>(null);
const panel = ref<HTMLElement | null>(null);
const dragging = ref(false);
const offset = ref(0);
let startY = 0;
let startScroll = 0;
let previouslyFocused: Element | null = null;
let openCount = 0;

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusables(): HTMLElement[] {
  if (!panel.value) return [];
  return Array.from(panel.value.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.hidden && el.getAttribute("aria-hidden") !== "true" && !el.closest("[hidden]"),
  );
}

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    emit("close");
    return;
  }
  if (event.key !== "Tab") return;
  const items = focusables();
  if (items.length === 0) {
    event.preventDefault();
    return;
  }
  const first = items[0];
  const last = items[items.length - 1];
  const active = document.activeElement as HTMLElement | null;
  if (event.shiftKey && (active === first || !panel.value?.contains(active))) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

function scrollBody(): HTMLElement | null {
  return panel.value?.querySelector<HTMLElement>(".sheet-body") ?? null;
}

function onPointerDown(event: PointerEvent) {
  if (event.pointerType === "mouse" && event.button !== 0) return;
  startY = event.clientY;
  startScroll = scrollBody()?.scrollTop ?? 0;
  dragging.value = false;
  offset.value = 0;
  window.addEventListener("pointermove", onPointerMove, { passive: false });
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
}

function onPointerMove(event: PointerEvent) {
  const delta = event.clientY - startY;
  const body = scrollBody();
  const target = event.target as HTMLElement | null;
  const inBody = body?.contains(target) ?? false;
  // Only start a dismiss drag when the body is at its top (or the touch is on the handle/header).
  if (delta > 0 && (!inBody || startScroll <= 0)) {
    if (!dragging.value && delta > 6) dragging.value = true;
    if (dragging.value) {
      offset.value = delta;
      event.preventDefault();
    }
  }
}

function onPointerUp() {
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
  const shouldClose = offset.value > 96;
  dragging.value = false;
  offset.value = 0;
  if (shouldClose) emit("close");
}

onMounted(async () => {
  previouslyFocused = document.activeElement;
  openCount = Number(document.body.dataset.sheets ?? "0") + 1;
  document.body.dataset.sheets = String(openCount);
  document.body.classList.add("sheet-open");
  document.addEventListener("keydown", onKey, true);
  await nextTick();
  const items = focusables();
  const preferred =
    panel.value?.querySelector<HTMLElement>("[data-autofocus]") ??
    items.find((el) => !el.classList.contains("sheet-close")) ??
    items[0];
  preferred?.focus({ preventScroll: true });
});

onBeforeUnmount(() => {
  document.removeEventListener("keydown", onKey, true);
  const remaining = Math.max(0, Number(document.body.dataset.sheets ?? "1") - 1);
  document.body.dataset.sheets = String(remaining);
  if (remaining === 0) document.body.classList.remove("sheet-open");
  if (previouslyFocused instanceof HTMLElement) previouslyFocused.focus({ preventScroll: true });
});
</script>

<template>
  <div ref="root" class="sheet-backdrop" @click.self="emit('close')">
    <div
      ref="panel"
      class="sheet"
      :class="{ dragging }"
      role="dialog"
      aria-modal="true"
      :aria-label="props.title"
      :style="offset ? { transform: `translateY(${offset}px)` } : undefined"
    >
      <div class="sheet-grab" @pointerdown="onPointerDown">
        <div class="sheet-handle" aria-hidden="true" />
        <div class="sheet-head">
          <div style="min-width: 0">
            <h2>{{ props.title }}</h2>
            <p v-if="props.subtitle" class="muted fine clamp-2">{{ props.subtitle }}</p>
          </div>
          <div class="actions">
            <slot name="head-actions" />
            <button
              v-if="!hideClose"
              class="icon-btn sheet-close"
              type="button"
              :aria-label="closeLabel ?? 'Close'"
              @click="emit('close')"
            >
              <NavIcon name="close" />
            </button>
          </div>
        </div>
      </div>
      <div class="sheet-body" @pointerdown="onPointerDown">
        <slot />
      </div>
      <div v-if="$slots.foot" class="sheet-foot">
        <slot name="foot" />
      </div>
    </div>
  </div>
</template>
