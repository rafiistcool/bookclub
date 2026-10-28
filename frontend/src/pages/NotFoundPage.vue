<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
  <section class="empty page-missing">
    <h1>{{ t("notFound.title") }}</h1>
    <p>{{ t("notFound.body") }}</p>
    <div class="btn-row">
      <RouterLink class="btn btn-primary" to="/">{{ t("notFound.toHome") }}</RouterLink>
      <RouterLink class="btn btn-ghost" to="/discover">{{ t("notFound.toDiscover") }}</RouterLink>
    </div>
  </section>
</template>

<style scoped>
.page-missing h1 {
  font-size: var(--text-xl);
}
</style>
