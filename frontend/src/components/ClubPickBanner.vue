<script setup lang="ts">
import { computed } from "vue";
import { STATUS_SHORT, type Status } from "../constants";
import type { ClubPick } from "../types";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  pick: ClubPick | null;
  compact?: boolean;
  emptyHint?: string;
}>();

const emit = defineEmits<{
  add: [];
  move: [];
  open: [];
}>();

const actionLabel = computed(() => {
  if (!props.pick) return "";
  if (props.pick.on_shelf) return "Move";
  return "Add to shelf";
});

function onAction() {
  if (!props.pick) return;
  if (props.pick.on_shelf) emit("move");
  else emit("add");
}
</script>

<template>
  <section class="club-pick-banner" :class="{ compact }">
    <template v-if="pick">
      <button class="club-pick-main" type="button" @click="emit('open')">
        <BookCover :title="pick.book.title" :cover-id="pick.book.cover_id" size="M" />
        <div class="book-meta">
          <p class="club-pick-kicker">Club pick</p>
          <h2>{{ pick.book.title }}</h2>
          <p v-if="pick.book.authors">{{ pick.book.authors }}</p>
          <p class="fine muted">
            Chosen by {{ pick.set_by }}
            <span v-if="pick.on_shelf"> · {{ STATUS_SHORT[pick.on_shelf as Status] }}</span>
          </p>
          <p v-if="pick.meeting_label" class="club-pick-meeting">{{ pick.meeting_label }}</p>
        </div>
      </button>
      <button class="btn" type="button" @click="onAction">{{ actionLabel }}</button>
    </template>
    <div v-else class="club-pick-empty">
      <p class="club-pick-kicker">Club pick</p>
      <p class="muted">{{ emptyHint || "No book chosen yet." }}</p>
    </div>
  </section>
</template>
