<script setup lang="ts">
import { useI18n } from "vue-i18n";
import DiaryFeed from "../components/DiaryFeed.vue";

/** The reader's own diary across books, newest first. Entries are written on a book's page. */
const { t } = useI18n();
</script>

<template>
  <section class="diary-page">
    <div class="page-head">
      <h1>{{ t("diaryPage.title") }}</h1>
      <p class="lede">{{ t("diaryPage.lede") }}</p>
    </div>
    <DiaryFeed mine :page-size="12" :heading="t('diaryPage.yours')" :empty-text="t('diaryPage.empty')">
      <template #empty>
        <RouterLink class="btn btn-ghost btn-sm" to="/shelf">{{ t("diaryPage.toShelf") }}</RouterLink>
      </template>
    </DiaryFeed>
  </section>
</template>
