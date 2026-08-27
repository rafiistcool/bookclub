<script setup lang="ts">
import { computed, ref } from "vue";
import { STATUS_SHORT, starLabel, type Status } from "../constants";
import type { SearchHit, ShelfItem } from "../types";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  item?: ShelfItem;
  hit?: SearchHit;
  readonly?: boolean;
  clubPickKey?: string | null;
}>();

const emit = defineEmits<{
  add: [];
  move: [];
  remove: [];
  clubPick: [];
  nominate: [];
  progress: [];
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
      <span v-if="item?.book && clubPickKey && item.book.ol_work_key === clubPickKey" class="badge club-pick">Club</span>
      <span v-if="badge" class="badge" :class="badge">{{ STATUS_SHORT[badge] }}</span>
      <p v-if="item?.rating" class="finish-note">{{ starLabel(item.rating) }}</p>
      <p v-if="item?.take" class="finish-note">{{ item.take }}</p>
      <p v-if="item?.dnf_reason" class="finish-note">{{ item.dnf_reason }}</p>
      <p v-if="item && item.progress != null" class="finish-note">{{ item.progress }}%</p>
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
        <button
          v-if="item?.status === 'currently_reading'"
          type="button"
          @click="menuOpen = false; emit('progress')"
        >
          Set progress
        </button>
        <button type="button" @click="menuOpen = false; emit('clubPick')">Set as club pick</button>
        <button type="button" @click="menuOpen = false; emit('nominate')">Nominate for next up</button>
        <button type="button" @click="menuOpen = false; emit('remove')">Remove</button>
      </div>
    </div>
  </article>
</template>
