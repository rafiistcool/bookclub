<script setup lang="ts">
import { ref, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";

const props = defineProps<{
  items: ShelfItem[]; month?: string; readonly?: boolean; disabled?: boolean;
  clubPickKey?: string | null; timezone: string; emptyHint: string;
}>();
const emit = defineEmits<{
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  moved: [item: ShelfItem, rows: ShelfItem[], month?: string];
}>();
const rows = ref<ShelfItem[]>([]);
watch(() => props.items, items => { rows.value = [...items]; }, { immediate: true, deep: true });
function moved(event: { newDraggableIndex?: number; newIndex?: number }) {
  if (props.readonly || props.disabled) return;
  const item = rows.value[event.newDraggableIndex ?? event.newIndex ?? 0];
  if (item) emit("moved", item, rows.value, props.month);
}
</script>

<template>
  <VueDraggable v-model="rows" class="book-grid shelf-grid" :class="{ 'empty-month': !rows.length }"
    :data-empty-hint="emptyHint" :data-month="month"
    :group="month === undefined ? 'shelf-active' : 'shelf-finished'"
    :animation="180" :disabled="readonly || disabled" :delay="250" :delay-on-touch-only="true"
    :touch-start-threshold="5" :fallback-tolerance="5"
    :prevent-on-filter="false" filter=".no-drag" draggable=".shelf-card"
    ghost-class="card-ghost" chosen-class="card-chosen" @add="moved" @update="moved">
    <BookCard v-for="item in rows" :key="item.id" :data-id="item.id"
      :timezone="timezone" :item="item" :readonly="readonly" :disabled="disabled" :club-pick-key="clubPickKey"
      @action="emit('action', item, $event)" />
  </VueDraggable>
</template>

<style scoped>
.shelf-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); align-items: stretch; gap: var(--space-6) var(--space-5); }
.empty-month { min-height: 88px; border: 1px dashed var(--border); border-radius: var(--radius-sm); }
.empty-month::after { content: attr(data-empty-hint); grid-column: 1 / -1; align-self: center; text-align: center; font-size: var(--text-sm); color: var(--text-muted); pointer-events: none; }
.card-ghost { opacity: 0.3; }
.card-chosen { cursor: grabbing; }
@media (max-width: 359px) {
  .shelf-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@media (min-width: 720px) {
  .shelf-grid { grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); }
}
</style>
