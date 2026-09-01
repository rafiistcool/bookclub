<script setup lang="ts">
import { computed } from "vue";
import { STATUS_LABEL } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useShelf } from "../stores/shelf";
import type { ShelfItem } from "../types";
import BookCover from "./BookCover.vue";
import NavIcon from "./NavIcon.vue";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  item: ShelfItem;
}>();

const emit = defineEmits<{
  close: [];
}>();

const flow = useFlow();
const pick = usePick();
const shelf = useShelf();
const book = computed(() => toRef(props.item.book));
const isClubPick = computed(() => pick.pick?.book.ol_work_key === props.item.book.ol_work_key);
const canMoveUp = computed(() => props.item.position > 0);

async function moveToTop() {
  await flow.applyStatus(book.value, props.item, props.item.status, 0);
}

void shelf;
</script>

<template>
  <Sheet :title="item.book.title" :subtitle="item.book.authors" @close="emit('close')">
    <div class="pick-hero" style="grid-template-columns: 56px 1fr; margin-bottom: 12px">
      <BookCover :title="item.book.title" :cover-id="item.book.cover_id" size="md" />
      <div class="meta">
        <span class="badge" :class="item.status" style="justify-self: start">{{ STATUS_LABEL[item.status] }}</span>
        <p v-if="item.book.year" class="fine muted">{{ item.book.year }}</p>
      </div>
    </div>
    <div class="status-list">
      <button class="btn" type="button" data-autofocus @click="flow.push({ kind: 'details', book })">
        Details
      </button>
      <button class="btn btn-primary" type="button" @click="flow.push({ kind: 'status', book, item, title: 'Move to…' })">
        Move to…
      </button>
      <button
        v-if="item.status === 'currently_reading'"
        class="btn"
        type="button"
        @click="flow.push({ kind: 'progress', item })"
      >
        Set progress
      </button>
      <button
        v-if="item.status === 'finished' || item.status === 'did_not_finish'"
        class="btn"
        type="button"
        @click="flow.push({ kind: 'finish', book, item, status: item.status, position: item.position })"
      >
        Edit rating and take
      </button>
      <button v-if="canMoveUp" class="btn" type="button" @click="moveToTop">Move to top</button>
      <button class="btn" type="button" @click="flow.push({ kind: 'quote', book })">
        <NavIcon name="quote" :size="18" /> Save a quote
      </button>
      <button v-if="!isClubPick" class="btn" type="button" @click="flow.push({ kind: 'clubPick', book })">
        Set as club pick
      </button>
      <button v-if="!isClubPick" class="btn" type="button" @click="flow.nominate(book)">Nominate for next up</button>
      <button class="btn btn-danger" type="button" @click="flow.push({ kind: 'remove', item })">Remove from shelf</button>
    </div>
  </Sheet>
</template>
