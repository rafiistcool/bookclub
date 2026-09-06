<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

defineProps<{ title: string }>();

const emit = defineEmits<{ close: [] }>();

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

onMounted(() => {
  restoreFocusTo = document.activeElement as HTMLElement | null;
  document.addEventListener("keydown", onKeydown, true);
  document.body.style.overflow = "hidden";
  (focusables()[0] ?? panel.value)?.focus();
});

onUnmounted(() => {
  document.removeEventListener("keydown", onKeydown, true);
  document.body.style.overflow = "";
  restoreFocusTo?.focus?.();
});
</script>

<template>
  <div class="sheet-backdrop" @click.self="emit('close')">
    <div
      ref="panel"
      class="sheet"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
      tabindex="-1"
    >
      <h2>{{ title }}</h2>
      <slot />
    </div>
  </div>
</template>

<style scoped>
.sheet-backdrop {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  z-index: 40;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  animation: fade var(--dur-base) var(--ease);
}

.sheet {
  width: min(100%, 480px);
  max-height: 90dvh;
  overflow-y: auto;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-xl) var(--radius-xl) 0 0;
  padding: var(--space-5) var(--space-4)
    calc(var(--space-5) + env(safe-area-inset-bottom));
  box-shadow: var(--elev-float);
  animation: rise var(--dur-base) var(--ease);
}

.sheet h2 {
  margin-bottom: var(--space-4);
}

@keyframes fade {
  from {
    opacity: 0;
  }
}

@keyframes rise {
  from {
    transform: translateY(14px);
  }
}

@media (min-width: 720px) {
  .sheet-backdrop {
    align-items: center;
  }

  .sheet {
    border-radius: var(--radius-xl);
    padding-bottom: var(--space-5);
  }
}
</style>
