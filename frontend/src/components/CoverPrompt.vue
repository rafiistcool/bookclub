<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { BookDetailOut, BookOut } from "../api/generated";
import { asksForCoverPhoto } from "../covers";
import CoverPhotoButton from "./CoverPhotoButton.vue";

/**
 * The book page's request for a photo (#74): a catalog or imported book
 * without a cover that the viewer has on their shelf. The photo goes up as
 * the viewer's edition's cover and fills the book's empty one, for everyone
 * (`POST /api/books/works/{work_id}/cover`). Shows nothing otherwise; the
 * small "Add a photo of your copy" link under the cover is there instead.
 */
const props = defineProps<{
  book: Pick<BookDetailOut, "id" | "edition_id" | "custom" | "on_shelf" | "cover_url" | "cover_thumb_url">;
}>();
const emit = defineEmits<{ covered: [book: BookOut] }>();

const { t } = useI18n();
const shown = computed(() => asksForCoverPhoto(props.book));
</script>

<template>
  <aside v-if="shown" class="cover-prompt" aria-labelledby="cover-prompt-title">
    <h2 id="cover-prompt-title" class="cover-prompt-title">{{ t("cover.prompt.title") }}</h2>
    <p class="fine muted">{{ t("cover.prompt.body") }}</p>
    <CoverPhotoButton
      :work-id="book.id"
      :edition-id="book.edition_id"
      :has-cover="false"
      :label="t('cover.prompt.action')"
      look="primary"
      @covered="(covered) => emit('covered', covered)"
    />
  </aside>
</template>

<style scoped>
.cover-prompt {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
  padding: var(--space-4);
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-lg);
  background: var(--surface-2);
}

.cover-prompt-title {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-lg);
}

.cover-prompt p {
  margin: 0;
  max-width: 60ch;
}
</style>
