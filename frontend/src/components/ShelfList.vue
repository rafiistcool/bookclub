<script setup lang="ts">
/** Phone layout: segmented control over one list. Reordering is via "Move to top". */
import { computed, ref, watch } from "vue";
import { STATUSES, STATUS_LABEL, STATUS_SHORT, type Status } from "../constants";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";
import SegmentedControl from "./SegmentedControl.vue";

const props = defineProps<{
  items: ShelfItem[];
  readonly?: boolean;
  clubPickKey?: string | null;
  initial?: Status;
}>();

const active = ref<Status>(props.initial ?? "currently_reading");

const counts = computed(() => {
  const map: Record<Status, number> = { want_to_read: 0, currently_reading: 0, finished: 0, did_not_finish: 0 };
  for (const item of props.items) map[item.status] += 1;
  return map;
});

const options = computed(() =>
  STATUSES.map((status) => ({ value: status, label: STATUS_SHORT[status], count: counts.value[status] })),
);

const visible = computed(() =>
  props.items
    .filter((item) => item.status === active.value)
    .sort((a, b) => a.position - b.position || a.id - b.id),
);

// Land on the first non-empty section when the preferred one is empty.
watch(
  () => props.items.length,
  () => {
    if (counts.value[active.value] === 0) {
      const first = STATUSES.find((status) => counts.value[status] > 0);
      if (first) active.value = first;
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="shelf-toolbar">
    <SegmentedControl
      :model-value="active"
      :options="options"
      label="Shelf section"
      @update:model-value="active = $event as Status"
    />
  </div>
  <div v-if="visible.length === 0" class="empty">
    <p>Nothing in {{ STATUS_LABEL[active] }} yet.</p>
  </div>
  <div v-else class="list" role="tabpanel">
    <BookCard
      v-for="item in visible"
      :key="item.id"
      :item="item"
      :club-pick-key="clubPickKey"
      :readonly="readonly"
    />
  </div>
</template>
