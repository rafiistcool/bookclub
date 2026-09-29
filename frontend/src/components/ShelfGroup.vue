<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { VueDraggable } from "vue-draggable-plus";
import { clubDate, dropPosition, type ShelfGroup } from "../shelf";
import type { ShelfItem } from "../types";
import type { Status } from "../constants";
import BookCard from "./BookCard.vue";

const props = defineProps<{
  group: ShelfGroup; items: ShelfItem[]; timezone: string;
  grid?: boolean; organizing?: boolean; readonly?: boolean; disabled?: boolean;
  clubPickKey?: string | null;
}>();
const emit = defineEmits<{
  dropped: [item: ShelfItem, status: Status, position: number];
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  remove: [item: ShelfItem]; clubPick: [item: ShelfItem]; nominate: [item: ShelfItem];
}>();
const { t, locale } = useI18n();
const rows = ref<ShelfItem[]>([]);
watch(() => props.group.items, (items) => { rows.value = [...items]; }, { immediate: true, deep: true });
const monthLabel = computed(() => props.group.month
  ? new Intl.DateTimeFormat(locale.value, { month: "long", year: "numeric", timeZone: "UTC" })
    .format(new Date(`${props.group.month}-01T12:00:00Z`))
  : t("shelf.noReadingDate"));
const dragGroup = computed(() => ({
  name: "shelf",
  put: (_to: unknown, _from: unknown, element: HTMLElement) => {
    if (props.disabled || (props.grid && !props.organizing)) return false;
    if (props.group.status !== "finished") return true;
    const item = props.items.find((row) => String(row.id) === element.dataset.id);
    if (!item) return false;
    const month = item.status === "finished"
      ? clubDate(item.finished_at, props.timezone).slice(0, 7)
      : clubDate(new Date().toISOString(), props.timezone).slice(0, 7);
    return month === props.group.month;
  },
}));
function dropped(event: { newIndex?: number }) {
  const item = rows.value[event.newIndex ?? 0];
  if (item) emit("dropped", item, props.group.status, dropPosition(props.items, rows.value, item, props.group.status));
}
</script>

<template>
  <div class="shelf-group">
    <h4 v-if="group.status === 'finished'" class="month-heading">{{ monthLabel }} · {{ group.items.length }}</h4>
    <VueDraggable v-model="rows" :class="grid ? 'book-grid' : 'column-body'"
      :group="dragGroup" :animation="180" filter=".no-drag"
      :handle="grid ? '.drag-handle' : undefined"
      :disabled="readonly || disabled || (grid && !organizing)"
      ghost-class="card-ghost" @add="dropped" @update="dropped">
      <BookCard v-for="item in rows" :key="item.id" :data-id="item.id"
        :item="item" :grid="grid" :readonly="readonly" :organizing="organizing"
        :disabled="disabled" :club-pick-key="clubPickKey"
        @action="emit('action', item, $event)" @remove="emit('remove', item)"
        @club-pick="emit('clubPick', item)" @nominate="emit('nominate', item)" />
    </VueDraggable>
    <p v-if="!rows.length" class="fine subtle">{{ readonly || grid && !organizing ? t('shelf.nothingHere') : t('shelf.dragHere') }}</p>
  </div>
</template>

<style scoped>
.month-heading { margin: var(--space-3) 0; font-size: var(--text-sm); }
.column-body { display: flex; flex-direction: column; gap: var(--space-2); min-height: 60px; }
.book-grid { min-height: 60px; }
.card-ghost { opacity: 0.35; }
.shelf-group + .shelf-group { margin-top: var(--space-5); }
</style>
