<script setup lang="ts">
import { bookPath, STATUS_SHORT, type Status } from "../constants";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  olWorkKey: string;
  title: string;
  authors?: string;
  coverId?: number | null;
  status?: Status | null;
  clubPick?: boolean;
  showAuthors?: boolean;
}>();
</script>

<template>
  <RouterLink class="book-tile" :to="bookPath(olWorkKey)">
    <span class="book-tile-cover">
      <BookCover :title="title" :cover-id="coverId" />
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
  </RouterLink>
</template>
