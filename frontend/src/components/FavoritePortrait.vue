<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { bookPath } from "../constants";
import type { Favorite } from "../types";
import BookCover from "./BookCover.vue";

const { t } = useI18n();

defineProps<{
  items: Favorite[];
  emptyHint?: string;
}>();
</script>

<template>
  <div v-if="items.length" class="portrait" role="list" :aria-label="t('favorites.portraitLabel')">
    <RouterLink
      v-for="row in items"
      :key="row.book.id"
      class="portrait-slot"
      role="listitem"
      :to="bookPath(row.book.ol_work_key)"
      :aria-label="t('favorites.slot', { n: row.position, title: row.book.title })"
    >
      <span class="portrait-rank nums" aria-hidden="true">{{ row.position }}</span>
      <BookCover
        :title="row.book.title"
        :cover-id="row.book.cover_id"
        :work-key="row.book.ol_work_key"
        :image-url="row.book.cover_url"
        size="md"
        eager
      />
    </RouterLink>
  </div>
  <p v-else-if="emptyHint" class="fine subtle portrait-empty">{{ emptyHint }}</p>
</template>

<style scoped>
.portrait {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
  margin: 0 0 var(--space-5);
}

.portrait-slot {
  display: grid;
  justify-items: center;
  gap: var(--space-1);
  text-decoration: none;
  color: inherit;
}

.portrait-rank {
  font-size: var(--text-2xs);
  font-weight: 700;
  letter-spacing: var(--tracking-caps);
  color: var(--text-subtle);
}

.portrait-empty {
  margin: 0 0 var(--space-5);
}
</style>
