<script setup lang="ts">
import { reactive, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { STATUSES, STATUS_LABEL, type Status } from "../constants";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";

const props = defineProps<{
  items: ShelfItem[];
  readonly?: boolean;
  clubPickKey?: string | null;
}>();

const emit = defineEmits<{
  remove: [item: ShelfItem];
  clubPick: [item: ShelfItem];
  nominate: [item: ShelfItem];
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

watch(() => props.items, apply, { immediate: true, deep: true });

function onDrop(status: Status, event: { newIndex?: number }) {
  const index = event.newIndex ?? 0;
  const item = lists[status][index];
  if (!item) return;
  emit("dropped", item, status, index);
}
</script>

<template>
  <div class="board">
    <section v-for="status in STATUSES" :key="status" class="column">
      <header class="column-head">
        <h3>{{ STATUS_LABEL[status] }}</h3>
        <span class="fine subtle nums">{{ lists[status].length }}</span>
      </header>
      <VueDraggable
        v-if="!readonly"
        v-model="lists[status]"
        class="column-body"
        group="shelf"
        :animation="180"
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
          :club-pick-key="clubPickKey"
          @remove="emit('remove', item)"
          @club-pick="emit('clubPick', item)"
          @nominate="emit('nominate', item)"
        />
      </VueDraggable>
      <div v-else class="column-body">
        <BookCard
          v-for="item in lists[status]"
          :key="item.id"
          :item="item"
          :club-pick-key="clubPickKey"
          readonly
        />
      </div>
      <p v-if="lists[status].length === 0" class="finer subtle column-empty">
        {{ readonly ? "Nothing here." : "Drag a book here." }}
      </p>
    </section>
  </div>
</template>

<style scoped>
.board {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-3);
  align-items: start;
}

.column {
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--surface-2);
  border-radius: var(--radius-lg);
  padding: var(--space-3);
}

.column-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}

.column-head h3 {
  font-size: var(--text-md);
}

.column-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-height: 60px;
}

.column-empty {
  padding: var(--space-2) 0 0;
}

.card-ghost {
  opacity: 0.35;
}

.card-dragging {
  transform: rotate(1.5deg);
}
</style>
