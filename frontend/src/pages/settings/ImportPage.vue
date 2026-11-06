<script setup lang="ts">
import { useI18n } from "vue-i18n";
import BackLink from "../../components/BackLink.vue";
import GoodreadsImport from "../../components/GoodreadsImport.vue";

/**
 * `/settings/import`: the Goodreads import, a page of its own because the
 * shelf links here and a running import needs room for its progress and the
 * skipped rows.
 */
const { t } = useI18n();
</script>

<template>
  <section class="settings-sub">
    <BackLink :label="t('settings.back')" fallback="/settings" exact />
    <div class="page-head">
      <h1>{{ t("settings.importTitle") }}</h1>
    </div>
    <p class="fine muted import-hint">{{ t("settings.importBlurb") }}</p>
    <GoodreadsImport />
  </section>
</template>

<style scoped>
.settings-sub {
  max-width: 560px;
}

.import-hint {
  margin: calc(-1 * var(--space-3)) 0 var(--space-4);
}
</style>
