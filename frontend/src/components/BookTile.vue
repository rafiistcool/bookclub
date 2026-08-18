<script setup lang="ts">
import { computed } from "vue";
import { STATUS_SHORT } from "../constants";
import type { SearchHit } from "../types";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  hit: SearchHit;
}>();

const emit = defineEmits<{
  open: [];
}>();

const badge = computed(() => props.hit.on_shelf);
</script>

<template>
  <button
    type="button"
    class="book-tile"
    :aria-label="hit.title"
    @click="emit('open')"
  >
    <span class="book-tile-cover">
      <BookCover :title="hit.title" :cover-id="hit.cover_id" size="L" fluid />
      <span v-if="badge" class="badge book-tile-badge" :class="badge">
        {{ STATUS_SHORT[badge] }}
      </span>
    </span>
    <span class="book-tile-title">{{ hit.title }}</span>
  </button>
</template>
