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
/* The page's whole content, not an empty list inside one: no dashed box, and
   a reading width in the middle of a wide column. */
.page-missing {
  max-width: 560px;
  margin-inline: auto;
  border: 0;
}

.page-missing h1 {
  font-size: var(--text-xl);
}
</style>
