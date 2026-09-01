<script setup lang="ts">
import { computed } from "vue";
import { STATUS_LABEL, formatDate } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import type { ShelfItem } from "../types";
import BookCover from "./BookCover.vue";
import NavIcon from "./NavIcon.vue";
import ProgressBar from "./ProgressBar.vue";
import StarRating from "./StarRating.vue";

const props = defineProps<{
  item: ShelfItem;
  readonly?: boolean;
  draggable?: boolean;
  clubPickKey?: string | null;
  /** Show the status badge (off inside a column that already says it). */
  showStatus?: boolean;
}>();

const flow = useFlow();
const isClubPick = computed(() => Boolean(props.clubPickKey && props.item.book.ol_work_key === props.clubPickKey));

function openDetails() {
  flow.open({ kind: "details", book: toRef(props.item.book) });
}
</script>

<template>
  <article class="book-card" :class="{ draggable }">
    <button class="cover-btn" type="button" style="border: 0; padding: 0; background: none" :aria-label="`Details for ${item.book.title}`" @click="openDetails">
      <BookCover :title="item.book.title" :cover-id="item.book.cover_id" size="md" />
    </button>
    <div class="meta">
      <h3 class="clamp-2">
        <button type="button" style="all: unset; cursor: pointer" @click="openDetails">{{ item.book.title }}</button>
      </h3>
      <p class="clamp-1">
        {{ item.book.authors }}<template v-if="item.book.year && item.book.authors"> · </template>{{ item.book.year || "" }}
      </p>
      <div v-if="isClubPick || showStatus" class="badges">
        <span v-if="isClubPick" class="badge club">Club pick</span>
        <span v-if="showStatus" class="badge" :class="item.status">{{ STATUS_LABEL[item.status] }}</span>
      </div>
      <ProgressBar
        v-if="item.status === 'currently_reading' && item.progress != null"
        :value="item.progress"
        thin
        :label="`${item.book.title} progress`"
      />
      <p v-if="item.status === 'finished' && (item.rating || item.finished_at)" class="card-note" style="display: flex; gap: 8px; align-items: center">
        <StarRating v-if="item.rating" :value="item.rating" />
        <span v-if="item.finished_at" class="faint tiny">{{ formatDate(item.finished_at) }}</span>
      </p>
      <p v-if="item.take" class="card-note clamp-2">“{{ item.take }}”</p>
      <p v-if="item.dnf_reason" class="card-note clamp-2">{{ item.dnf_reason }}</p>
    </div>
    <button
      v-if="!readonly"
      class="icon-btn no-drag"
      type="button"
      :aria-label="`Actions for ${item.book.title}`"
      @click="flow.open({ kind: 'actions', item })"
    >
      <NavIcon name="more" />
    </button>
    <span v-else />
  </article>
</template>
