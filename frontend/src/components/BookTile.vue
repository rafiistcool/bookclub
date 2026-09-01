<script setup lang="ts">
import { STATUS_SHORT, type Status } from "../constants";
import BookCover from "./BookCover.vue";

defineProps<{
  title: string;
  authors?: string;
  coverId: number | null;
  status?: Status | null;
  clubPick?: boolean;
}>();

const emit = defineEmits<{
  open: [];
}>();
</script>

<template>
  <button type="button" class="book-tile" :aria-label="title" @click="emit('open')">
    <span class="book-tile-cover">
      <BookCover :title="title" :cover-id="coverId" size="fluid" />
      <span v-if="clubPick" class="badge on-cover club">Club</span>
      <span v-else-if="status" class="badge on-cover" :class="status">{{ STATUS_SHORT[status] }}</span>
    </span>
    <span class="book-tile-title clamp-2">{{ title }}</span>
    <span v-if="authors" class="book-tile-author clamp-1">{{ authors }}</span>
  </button>
</template>
