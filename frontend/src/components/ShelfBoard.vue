<script setup lang="ts">
/** Four-column drag-and-drop board. Used at ≥720px; phones get ShelfList. */
import { reactive, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { STATUSES, STATUS_LABEL, type Status } from "../constants";
import { useFlow } from "../stores/flow";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";

const props = defineProps<{
  items: ShelfItem[];
  readonly?: boolean;
  clubPickKey?: string | null;
}>();

const emit = defineEmits<{
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

// A drop into Finished/DNF opens a sheet; if it is dismissed without saving the
// optimistic column move must snap back to what the store says.
const flow = useFlow();
watch(
  () => flow.current,
  (current, previous) => {
    if (previous && !current) apply(props.items);
  },
);

function onDrop(status: Status, evt: { newIndex?: number }) {
  const index = evt.newIndex ?? 0;
  const item = lists[status][index];
  if (!item) return;
  emit("dropped", item, status, index);
}
</script>

<template>
  <div class="board">
    <section v-for="status in STATUSES" :key="status" class="column" :aria-label="STATUS_LABEL[status]">
      <header>
        <h2>{{ STATUS_LABEL[status] }}</h2>
        <span class="count">{{ lists[status].length }}</span>
      </header>
      <VueDraggable
        v-if="!readonly"
        v-model="lists[status]"
        class="column-body scroll"
        group="shelf"
        :animation="180"
        :delay="120"
        :delay-on-touch-only="true"
        :touch-start-threshold="5"
        filter=".no-drag"
        ghost-class="card-ghost"
        drag-class="card-dragging"
        @add="onDrop(status, $event)"
        @update="onDrop(status, $event)"
      >
        <BookCard v-for="item in lists[status]" :key="item.id" :item="item" :club-pick-key="clubPickKey" draggable />
      </VueDraggable>
      <div v-else class="column-body scroll">
        <BookCard v-for="item in lists[status]" :key="item.id" :item="item" :club-pick-key="clubPickKey" readonly />
      </div>
    </section>
  </div>
</template>
