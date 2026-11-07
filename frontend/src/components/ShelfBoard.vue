<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { droppedMonth, finishedDropPosition, flowMarkers, orderedShelf, readingMonthShort, shelfDropPosition, visibleShelf } from "../shelf";
import type { Status } from "../constants";
import type { ShelfItem } from "../types";
import ShelfGroup from "./ShelfGroup.vue";

const props = withDefaults(defineProps<{
  items: ShelfItem[]; readonly?: boolean; clubPickIds?: readonly number[];
  disabled?: boolean; filter?: "all" | Status; timezone?: string;
}>(), { filter: "all", timezone: "UTC" });
const emit = defineEmits<{
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  reordered: [item: ShelfItem, position: number, month?: string];
}>();
const { t, locale } = useI18n();
const rows = computed(() => orderedShelf(props.items));
const active = computed(() => rows.value.filter(item => item.status !== "finished" && (props.filter === "all" || item.status === props.filter)));
const showFinished = computed(() => props.filter === "all" || props.filter === "finished");
/** One grid for every finished book; a small month marker sits over the first book of each month. */
const finished = computed(() => (showFinished.value
  ? visibleShelf(props.items.filter(item => item.status === "finished"), "finished", props.timezone) : []));
/** While a finished book is dragged: the grid's order with it in its new slot. */
const dragging = ref<{ rows: ShelfItem[]; item: ShelfItem } | null>(null);
/**
 * Per book: the marker's words on the first of a month, "" on the rest.
 * During a drag they follow the slot, so the month shown there is the one
 * the drop gives the book, and the month it left keeps its marker.
 */
const markers = computed<Record<number, string>>(() => {
  const order = dragging.value?.rows ?? finished.value;
  const months = flowMarkers(order, props.timezone, dragging.value?.item);
  return Object.fromEntries(order.map((item, index) => {
    const month = months[index] ?? null;
    return [item.id, month === null ? "" : month ? readingMonthShort(month, locale.value) : t("shelf.undated")];
  }));
});

function moved(item: ShelfItem, visible: ShelfItem[]) {
  emit("reordered", item, shelfDropPosition(props.items, visible, item));
}
/** A drop among another month's books moves the book into that month, placed among that month's books. */
function movedFinished(item: ShelfItem, visible: ShelfItem[]) {
  dragging.value = null;
  const month = droppedMonth(visible, item, props.timezone);
  emit("reordered", item, finishedDropPosition(props.items, visible, item, month, props.timezone), month);
}
</script>

<template>
  <div class="shelf-books" :aria-busy="disabled">
    <h2 v-if="active.length" class="shelf-section-heading">{{ t(readonly ? 'shelf.currentMemberBooks' : 'shelf.currentBooks') }}</h2>
    <ShelfGroup v-if="active.length" :items="active" :readonly="readonly" :disabled="disabled"
      :timezone="timezone" :club-pick-ids="clubPickIds"
      @moved="moved" @action="(item, action) => emit('action', item, action)" />
    <section v-if="finished.length" class="finished-section" :class="{ 'has-active': active.length }">
      <h2 class="shelf-section-heading">{{ t('shelf.finishedBooks') }} <span class="fine subtle nums">{{ finished.length }}</span></h2>
      <ShelfGroup :items="finished" :markers="markers" :readonly="readonly" :disabled="disabled"
        :timezone="timezone" :club-pick-ids="clubPickIds"
        @moved="movedFinished" @preview="state => (dragging = state)" @action="(item, action) => emit('action', item, action)" />
    </section>
    <p v-if="!active.length && !finished.length" class="fine subtle">{{ t('shelf.nothingHere') }}</p>
  </div>
</template>

<style scoped>
.shelf-section-heading { display: flex; align-items: baseline; gap: var(--space-2); font-size: var(--text-xl); margin-bottom: var(--space-4); }
.shelf-section-heading .nums { font-family: var(--sans); font-weight: 400; letter-spacing: normal; }
.finished-section.has-active { margin-top: var(--space-7); }
</style>
