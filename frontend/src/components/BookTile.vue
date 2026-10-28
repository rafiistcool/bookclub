<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { bookPath, statusShort, starLabel, type CoverSize, type Status } from "../constants";
import type { CoverBook } from "../covers";
import BookCover from "./BookCover.vue";

const { t } = useI18n();

const props = defineProps<{
  book: CoverBook & { id: number };
  status?: Status | null;
  /** On the viewer's shelf, status unknown (search hits only say yes or no). */
  onShelf?: boolean;
  hideStatus?: boolean;
  clubPick?: boolean;
  showAuthors?: boolean;
  /** A short line under the title, such as a release date. */
  note?: string;
  /** The owner's stars, shown under the title for finished books. */
  rating?: number | null;
  size?: CoverSize;
  eager?: boolean;
}>();
</script>

<template>
  <RouterLink class="book-tile" :to="bookPath(book)">
    <span class="book-tile-cover">
      <BookCover :book="book" :size="size ?? 'tile'" :eager="eager" />
      <span v-if="clubPick" class="badge book-tile-badge club-pick">{{ t("common.club") }}</span>
      <span
        v-else-if="props.status && !hideStatus"
        class="badge book-tile-badge"
        :class="props.status"
      >
        {{ statusShort(props.status) }}
      </span>
      <span v-else-if="onShelf && !hideStatus" class="badge book-tile-badge on-shelf">
        {{ t("discover.onShelfBadge") }}
      </span>
    </span>
    <span class="book-tile-title">{{ book.title }}</span>
    <span
      v-if="rating && status === 'finished'"
      class="book-tile-sub stars"
      :aria-label="t('book.starsOf', { n: rating })"
    >
      {{ starLabel(rating) }}
    </span>
    <span v-if="showAuthors && book.authors" class="book-tile-sub">{{ book.authors }}</span>
    <span v-if="note" class="book-tile-sub book-tile-note">{{ note }}</span>
  </RouterLink>
</template>
