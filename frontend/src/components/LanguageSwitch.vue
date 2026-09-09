<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { LOCALES } from "../i18n";
import { useLocale } from "../stores/locale";

defineProps<{
  compact?: boolean;
}>();

const { t } = useI18n();
const locale = useLocale();
</script>

<template>
  <div class="segmented" role="group" :aria-label="t('auth.language')">
    <button
      v-for="id in LOCALES"
      :key="id"
      type="button"
      :aria-pressed="locale.locale === id"
      @click="locale.setLocale(id)"
    >
      {{ compact ? id.toUpperCase() : t(`locale.${id}`) }}
    </button>
  </div>
</template>
