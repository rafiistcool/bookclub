<script setup lang="ts">
import { computed, ref } from "vue";
import { STATUS_SHORT, type Status } from "../constants";
import type { SearchHit, ShelfItem } from "../types";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  item?: ShelfItem;
  hit?: SearchHit;
  readonly?: boolean;
}>();

const emit = defineEmits<{
  add: [];
  move: [];
  remove: [];
}>();

const menuOpen = ref(false);

const title = computed(() => props.item?.book.title ?? props.hit?.title ?? "");
const authors = computed(() => props.item?.book.authors ?? props.hit?.authors ?? "");
const year = computed(() => props.item?.book.year ?? props.hit?.year ?? null);
const coverId = computed(() => props.item?.book.cover_id ?? props.hit?.cover_id ?? null);
const badge = computed<Status | null>(
  () => props.item?.status ?? props.hit?.on_shelf ?? null,
);

function toggleMenu() {
  menuOpen.value = !menuOpen.value;
}
</script>

<template>
  <article class="book-card">
    <BookCover :title="title" :cover-id="coverId" size="M" />
    <div class="book-meta">
      <h3>{{ title }}</h3>
      <p v-if="authors">{{ authors }}</p>
      <p v-if="year" class="fine">{{ year }}</p>
      <span v-if="badge" class="badge" :class="badge">{{ STATUS_SHORT[badge] }}</span>
    </div>
    <div v-if="hit && !hit.on_shelf" class="no-drag">
      <button class="btn" type="button" @click="emit('add')">Add</button>
    </div>
    <div v-else-if="hit && hit.on_shelf" class="no-drag">
      <button class="btn btn-ghost" type="button" @click="emit('move')">Move</button>
    </div>
    <div v-else-if="!readonly" class="no-drag" style="position: relative">
      <button class="icon-btn" type="button" aria-label="Book actions" @click="toggleMenu">
        ···
      </button>
      <div v-if="menuOpen" class="menu" style="position: absolute; right: 0; top: 40px">
        <button type="button" @click="menuOpen = false; emit('move')">Move to…</button>
        <button type="button" @click="menuOpen = false; emit('remove')">Remove</button>
      </div>
    </div>
  </article>
</template>
