<script setup lang="ts">
import { ChevronDownIcon } from "@heroicons/vue/24/outline";

/**
 * A `<details>` of the admin page: a full-height summary line with a chevron
 * that turns on open, instead of the browser's triangle. `ruled` puts
 * hairlines above and below it, for one that stands between lists.
 */
defineProps<{ summary: string; ruled?: boolean }>();
</script>

<template>
  <details class="disclosure" :class="{ ruled }">
    <summary>
      <span class="disclosure-label">{{ summary }}</span>
      <ChevronDownIcon class="disclosure-chevron" aria-hidden="true" />
    </summary>
    <div class="disclosure-body">
      <slot />
    </div>
  </details>
</template>

<style scoped>
.disclosure.ruled {
  border-block: 1px solid var(--border);
}

summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  min-height: var(--tap);
  list-style: none;
  cursor: pointer;
  color: var(--text-muted);
  font-size: var(--text-sm);
  font-weight: 600;
  transition: transform var(--dur-fast) var(--ease-out);
}

summary::-webkit-details-marker {
  display: none;
}

summary:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  summary:hover {
    color: var(--text);
  }
}

.disclosure[open] > summary {
  color: var(--text);
}

.disclosure-label {
  min-width: 0;
}

.disclosure-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
  transition: transform var(--dur-fast) var(--ease-out);
}

.disclosure[open] .disclosure-chevron {
  transform: rotate(180deg);
}

.disclosure-body {
  padding-bottom: var(--space-3);
}
</style>
