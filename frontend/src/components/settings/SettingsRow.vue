<script setup lang="ts">
import { ChevronRightIcon } from "@heroicons/vue/24/outline";
import { computed, type Component } from "vue";
import { RouterLink } from "vue-router";

/**
 * One row of a settings list (the sheet-row style of ClubMoreSheet): an icon,
 * the label, the current value right-aligned and muted, and a chevron. With
 * `to` it is a link, with `href` a plain link (the data export's download),
 * otherwise a button that emits `click`. A changed value crossfades in, so a
 * choice made in a sheet shows behind it without a toast.
 */
const props = defineProps<{
  icon: Component;
  label: string;
  value?: string;
  to?: string;
  href?: string;
  download?: boolean;
  danger?: boolean;
  /** A second line under the label, in full (an address too long for the value's place). */
  detail?: string;
  /** Something needs the reader here (an unconfirmed address): a dot, and this text for screen readers. */
  alert?: string;
  /** A row that acts at once (sign out, download) has no chevron. */
  trailing?: Component | null;
}>();
const emit = defineEmits<{ click: [] }>();

const tag = computed(() => (props.to ? RouterLink : props.href ? "a" : "button"));
const attrs = computed(() => {
  if (props.to) return { to: props.to };
  if (props.href) return { href: props.href, download: props.download ? "" : undefined };
  return { type: "button" };
});
const end = computed(() => (props.trailing === undefined ? (props.danger ? null : ChevronRightIcon) : props.trailing));
</script>

<template>
  <component :is="tag" class="settings-row" :class="{ danger }" v-bind="attrs" @click="emit('click')">
    <component :is="icon" class="row-icon" aria-hidden="true" />
    <span v-if="detail" class="row-label row-label-stack">
      {{ label }}
      <span class="row-detail">{{ detail }}</span>
    </span>
    <span v-else class="row-label">{{ label }}</span>
    <span v-if="value || alert || $slots.value" class="row-value">
      <slot name="value">
        <Transition v-if="value" name="row-value" mode="out-in">
          <span :key="value" class="row-value-text">{{ value }}</span>
        </Transition>
      </slot>
      <span v-if="alert" class="row-alert">
        <span class="visually-hidden">{{ alert }}</span>
      </span>
    </span>
    <component :is="end" v-if="end" class="row-chevron" aria-hidden="true" />
  </component>
</template>

<style scoped>
.settings-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  min-height: 52px;
  padding: 0 var(--space-1);
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text);
  font-weight: 600;
  text-align: left;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.settings-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .settings-row:hover .row-label {
    color: var(--accent);
  }

  .settings-row.danger:hover .row-label {
    color: var(--danger);
    text-decoration: underline;
    text-underline-offset: 0.18em;
  }
}

.row-icon {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--text-muted);
}

.row-label {
  flex: 1 0 auto;
  min-width: 0;
}

.row-label-stack {
  display: grid;
  gap: 2px;
  flex-shrink: 1;
  padding: var(--space-2) 0;
}

.row-detail {
  color: var(--text-muted);
  font-size: var(--text-sm);
  font-weight: 400;
  overflow-wrap: anywhere;
}

.row-value {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-2);
  min-width: 0;
  flex: 0 1 auto;
  max-width: 62%;
  color: var(--text-muted);
  font-weight: 400;
}

.row-value-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-alert {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.row-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}

.danger,
.danger .row-icon {
  color: var(--danger);
}

.row-value-enter-active,
.row-value-leave-active {
  transition:
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-base) var(--ease-out);
}

.row-value-leave-active {
  transition-duration: var(--dur-fast);
}

.row-value-enter-from {
  opacity: 0;
  transform: translateY(4px);
}

.row-value-leave-to {
  opacity: 0;
}
</style>
