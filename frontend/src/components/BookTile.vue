<script setup lang="ts">
import { bookPath, STATUS_SHORT, starLabel, type CoverSize, type Status } from "../constants";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  olWorkKey: string;
  title: string;
  authors?: string;
  coverId?: number | null;
  coverEditionKey?: string | null;
  isbn?: string | null;
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
        :size="size ?? 'tile'"
        :eager="eager"
      />
      <span v-if="clubPick" class="badge book-tile-badge club-pick">Club</span>
      <span
        v-else-if="props.status"
        class="badge book-tile-badge"
        :class="props.status"
      >
        {{ STATUS_SHORT[props.status] }}
      </span>
    </span>
    <span class="book-tile-title">{{ title }}</span>
    <span v-if="showAuthors && authors" class="book-tile-sub">{{ authors }}</span>
    <span
      v-if="rating && status === 'finished'"
      class="book-tile-sub stars"
      :aria-label="`Rated ${rating} of 5`"
    >
      {{ starLabel(rating) }}
    </span>
  </RouterLink>
</template>
