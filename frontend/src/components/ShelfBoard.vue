<script setup lang="ts">
import { reactive, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { STATUSES, STATUS_LABEL, type Status } from "../constants";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";

const props = defineProps<{
  items: ShelfItem[];
  readonly?: boolean;
}>();

const emit = defineEmits<{
  move: [item: ShelfItem];
  remove: [item: ShelfItem];
  dropped: [item: ShelfItem, status: Status, position: number];
}>();

const lists = reactive<Record<Status, ShelfItem[]>>({
  want_to_read: [],
  currently_reading: [],
  finished: [],
  did_not_finish: [],
});

function apply(items: ShelfItem[]) {
  for (const status of STATUSES) lists[status] = [];
  const sorted = [...items].sort((a, b) => a.position - b.position || a.id - b.id);
  for (const item of sorted) lists[item.status].push(item);
}

watch(
  () => props.items,
  (items) => apply(items),
  { immediate: true, deep: true },
);

function onDrop(status: Status, evt: { newIndex?: number }) {
  const index = evt.newIndex ?? 0;
  const item = lists[status][index];
  if (!item) return;
  emit("dropped", item, status, index);
}
</script>

<template>
  <p v-if="!readonly" class="drag-hint">
    Drag a book to another column — or hold briefly on a phone. You can also use Move to…
  </p>
  <div class="board">
    <section v-for="status in STATUSES" :key="status" class="column">
      <header>
        <h2>{{ STATUS_LABEL[status] }}</h2>
        <span class="count">{{ lists[status].length }}</span>
      </header>
      <VueDraggable
        v-if="!readonly"
        v-model="lists[status]"
        class="column-body"
        group="shelf"
        :animation="180"
        :delay="180"
        :delay-on-touch-only="true"
        :touch-start-threshold="5"
        filter=".no-drag"
        ghost-class="card-ghost"
        drag-class="card-dragging"
        @add="onDrop(status, $event)"
        @update="onDrop(status, $event)"
      >
        <BookCard
          v-for="item in lists[status]"
          :key="item.id"
          :item="item"
          @move="emit('move', item)"
          @remove="emit('remove', item)"
        />
      </VueDraggable>
      <div v-else class="column-body">
        <BookCard v-for="item in lists[status]" :key="item.id" :item="item" readonly />
      </div>
    </section>
  </div>
</template>
