<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { STATUSES, statusLabel, type Status } from "../constants";
import { shelfGroups } from "../shelf";
import type { ShelfItem } from "../types";
import ShelfGroup from "./ShelfGroup.vue";

const props = withDefaults(defineProps<{
  items: ShelfItem[]; readonly?: boolean; clubPickKey?: string | null;
  timezone?: string; grid?: boolean; organizing?: boolean; disabled?: boolean;
  filter?: "all" | Status;
}>(), { timezone: "UTC", filter: "all" });
const emit = defineEmits<{
  remove: [item: ShelfItem]; clubPick: [item: ShelfItem]; nominate: [item: ShelfItem];
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  dropped: [item: ShelfItem, status: Status, position: number];
}>();
const { t } = useI18n();
const statuses = computed(() => STATUSES.filter((status) => props.filter === "all" || props.filter === status));
const groups = computed(() => shelfGroups(props.items, props.timezone, !props.readonly && !props.grid));
</script>

<template>
  <div :class="grid ? 'shelf-grid-sections' : 'board'" :aria-busy="disabled">
    <section v-for="status in statuses" :key="status" class="column">
      <header class="column-head">
        <h3>{{ statusLabel(status) }}</h3>
        <span class="fine subtle nums">{{ items.filter(item => item.status === status).length }}</span>
      </header>
      <ShelfGroup v-for="group in groups.filter(group => group.status === status)" :key="group.key"
        :group="group" :items="items" :timezone="timezone" :grid="grid" :organizing="organizing"
        :readonly="readonly" :disabled="disabled" :club-pick-key="clubPickKey"
        @dropped="(item, target, position) => emit('dropped', item, target, position)"
        @action="(item, action) => emit('action', item, action)"
        @remove="emit('remove', $event)" @club-pick="emit('clubPick', $event)" @nominate="emit('nominate', $event)" />
      <p v-if="!groups.some(group => group.status === status)" class="fine subtle">{{ t('shelf.nothingHere') }}</p>
    </section>
  </div>
</template>

<style scoped>
.board { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: var(--space-3); align-items: start; }
.column { min-width: 0; }
.board .column { background: var(--surface-2); border-radius: var(--radius-lg); padding: var(--space-3); }
.column-head { display: flex; align-items: baseline; justify-content: space-between; gap: var(--space-2); margin-bottom: var(--space-3); }
.column-head h3 { font-size: var(--text-md); }
.shelf-grid-sections { display: grid; gap: var(--space-6); }
</style>
