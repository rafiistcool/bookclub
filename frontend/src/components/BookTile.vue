<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { bookPath, statusShort, starLabel, type CoverSize, type Status } from "../constants";
import BookCover from "./BookCover.vue";

const { t } = useI18n();

const props = defineProps<{
  olWorkKey: string;
  title: string;
  authors?: string;
  coverId?: number | null;
  coverEditionKey?: string | null;
  isbn?: string | null;
  imageUrl?: string | null;
  status?: Status | null;
  clubPick?: boolean;
  showAuthors?: boolean;
  /** The owner's stars, shown under the title for finished books. */
  rating?: number | null;
  size?: CoverSize;
  eager?: boolean;
}>();
</script>

<template>
  <RouterLink class="book-tile" :to="bookPath(olWorkKey)">
    <span class="book-tile-cover">
      <BookCover
        :title="title"
        :cover-id="coverId"
        :cover-edition-key="coverEditionKey"
        :isbn="isbn"
        :work-key="olWorkKey"
        :image-url="imageUrl"
        :size="size ?? 'tile'"
        :eager="eager"
      />
      <span v-if="clubPick" class="badge book-tile-badge club-pick">{{ t("common.club") }}</span>
      <span
        v-else-if="props.status"
        class="badge book-tile-badge"
        :class="props.status"
      >
        {{ statusShort(props.status) }}
      </span>
    </span>
    <span class="book-tile-title">{{ title }}</span>
    <span v-if="showAuthors && authors" class="book-tile-sub">{{ authors }}</span>
    <span
      v-if="rating && status === 'finished'"
      class="book-tile-sub stars"
      :aria-label="t('book.starsOf', { n: rating })"
    >
      {{ starLabel(rating) }}
    </span>
  </RouterLink>
</template>
