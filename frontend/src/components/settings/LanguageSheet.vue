<script setup lang="ts">
import { CheckIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "vue-i18n";
import { LOCALES, type Locale } from "../../i18n";
import { useLocale } from "../../stores/locale";
import BottomSheet from "../BottomSheet.vue";

/** The app's language: one tap picks it, and the sheet closes on the new language. */
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const locale = useLocale();

function pick(id: Locale) {
  locale.setLocale(id);
  emit("close");
}
</script>

<template>
  <BottomSheet class="language-sheet" :title="t('settings.language')" @close="emit('close')">
    <ul class="choice-list">
      <li v-for="id in LOCALES" :key="id">
        <button
          class="choice-row"
          :class="{ current: locale.locale === id }"
          type="button"
          :lang="id"
          :aria-pressed="locale.locale === id"
          @click="pick(id)"
        >
          <span class="choice-label">{{ t(`locale.${id}`) }}</span>
          <CheckIcon v-if="locale.locale === id" class="choice-check" aria-hidden="true" />
        </button>
      </li>
    </ul>
  </BottomSheet>
</template>

<style scoped>
.choice-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.choice-list li + li {
  border-top: 1px solid var(--border);
}

.choice-row {
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
  transition: transform var(--dur-fast) var(--ease-out);
}

.choice-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .choice-row:hover .choice-label {
    color: var(--accent);
  }
}

.choice-label {
  flex: 1;
}

.current .choice-label {
  color: var(--accent);
}

.choice-check {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--accent);
}
</style>
