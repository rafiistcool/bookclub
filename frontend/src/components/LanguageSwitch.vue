<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { LOCALES } from "../i18n";
import { useLocale } from "../stores/locale";

/**
 * The language on the pages before sign-in. `compact` is the quiet form
 * there: two words in their own language, plain text beside the links, so
 * it never outweighs "Konto erstellen". Without it, a segmented control.
 */
defineProps<{
  compact?: boolean;
}>();

const { t } = useI18n();
const locale = useLocale();
</script>

<template>
  <div :class="compact ? 'locale-quiet' : 'segmented'" role="group" :aria-label="t('auth.language')">
    <button
      v-for="id in LOCALES"
      :key="id"
      type="button"
      :lang="id"
      :aria-pressed="locale.locale === id"
      @click="locale.choose(id)"
    >
      {{ t(`locale.${id}`) }}
    </button>
  </div>
</template>

<style scoped>
.locale-quiet {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  column-gap: var(--space-1);
  margin-left: calc(-1 * var(--space-2));
}

.locale-quiet button {
  min-height: var(--tap);
  padding: 0 var(--space-2);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--text-sm);
  cursor: pointer;
  text-decoration: underline;
  text-decoration-color: transparent;
  text-underline-offset: 0.25em;
  transition: color var(--dur-fast) var(--ease), text-decoration-color var(--dur-fast) var(--ease);
}

/* The language in use reads as the current one, not as a button to press. */
.locale-quiet button[aria-pressed="true"] {
  color: var(--text);
  font-weight: 600;
  cursor: default;
}

@media (hover: hover) {
  .locale-quiet button[aria-pressed="false"]:hover {
    color: var(--text);
    text-decoration-color: currentColor;
  }
}
</style>
