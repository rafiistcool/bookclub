<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { clubDate, orderedShelf, readingMonth, readingMonthLabel, shelfDropPosition } from "../shelf";
import type { Status } from "../constants";
import type { ShelfItem } from "../types";
import ShelfGroup from "./ShelfGroup.vue";

const props = withDefaults(defineProps<{
  items: ShelfItem[]; readonly?: boolean; clubPickKey?: string | null;
  disabled?: boolean; filter?: "all" | Status; timezone?: string;
}>(), { filter: "all", timezone: "UTC" });
const emit = defineEmits<{
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  reordered: [item: ShelfItem, position: number, month?: string];
}>();
const { t, locale } = useI18n();
const extraMonths = ref<string[]>([]);
const monthValue = ref("");
const showMonthPicker = ref(false);
const currentMonth = computed(() => clubDate(new Date().toISOString(), props.timezone).slice(0, 7));
const rows = computed(() => orderedShelf(props.items));
const active = computed(() => rows.value.filter(item => item.status !== "finished" && (props.filter === "all" || item.status === props.filter)));
const showFinished = computed(() => props.filter === "all" || props.filter === "finished");
const months = computed(() => {
  const finished = rows.value.filter(item => item.status === "finished");
  const keys = new Set(finished.map(item => readingMonth(item, props.timezone)));
  if (!props.readonly) {
    extraMonths.value.forEach(month => keys.add(month));
    // Keep an adjacent empty month available as a drop target, even on a new shelf.
    const latest = [...keys].filter(Boolean).sort().at(-1) || currentMonth.value;
    if (finished.length || props.filter === "finished") {
      if (!keys.size || keys.has("")) keys.add(latest);
      const [year, month] = latest.split("-").map(Number);
      keys.add(`${month === 1 ? year - 1 : year}-${String(month === 1 ? 12 : month - 1).padStart(2, "0")}`);
    }
  }
  return [...keys].sort((a, b) => b.localeCompare(a)).map(month => ({
    month, items: finished.filter(item => readingMonth(item, props.timezone) === month),
  }));
});
function addMonth() {
  if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(monthValue.value) || monthValue.value < "0001-01" || monthValue.value > currentMonth.value) return;
  extraMonths.value.push(monthValue.value);
  showMonthPicker.value = false;
}
function moved(item: ShelfItem, visible: ShelfItem[], month?: string) {
  const previousMonth = readingMonth(item, props.timezone);
  if (month !== undefined && previousMonth && month !== previousMonth) extraMonths.value.push(previousMonth);
  emit("reordered", item, shelfDropPosition(props.items, visible, item), month);
}
</script>

<template>
  <div class="shelf-books" :aria-busy="disabled">
    <h2 v-if="active.length" class="shelf-section-heading">{{ t(readonly ? 'shelf.currentMemberBooks' : 'shelf.currentBooks') }}</h2>
    <ShelfGroup v-if="active.length" :items="active" :readonly="readonly" :disabled="disabled"
      :timezone="timezone" :club-pick-key="clubPickKey" :empty-hint="t('shelf.nothingHere')"
      @moved="moved" @action="(item, action) => emit('action', item, action)" />
    <section v-if="showFinished" class="finished-section" :class="{ 'has-active': active.length }">
      <h2 v-if="months.length" class="shelf-section-heading">{{ t('shelf.finishedBooks') }}</h2>
      <section v-for="group in months" :key="group.month" class="shelf-month" :aria-label="group.month ? readingMonthLabel(group.month, locale) : t('shelf.noReadingDate')">
        <h3 class="month-heading">{{ group.month ? readingMonthLabel(group.month, locale) : t('shelf.noReadingDate') }} <span class="fine subtle nums">{{ group.items.length }}</span></h3>
        <ShelfGroup :items="group.items" :month="group.month" :readonly="readonly" :disabled="disabled"
          :timezone="timezone" :club-pick-key="clubPickKey" :empty-hint="t('shelf.dropIntoMonth')"
          @moved="moved" @action="(item, action) => emit('action', item, action)" />
      </section>
      <div v-if="!readonly" class="month-picker">
        <button v-if="!showMonthPicker" type="button" class="btn btn-ghost" :disabled="disabled"
          @click="showMonthPicker = true">{{ t('shelf.addMonth') }}</button>
        <form v-else class="month-picker-form" @submit.prevent="addMonth">
          <label for="shelf-month">{{ t('shelf.completionMonth') }}</label>
          <input id="shelf-month" v-model="monthValue" type="month" min="0001-01" :max="currentMonth" required :disabled="disabled" />
          <button type="submit" class="btn btn-ghost" :disabled="disabled || !monthValue || monthValue > currentMonth">{{ t('shelf.addMonth') }}</button>
        </form>
      </div>
    </section>
    <p v-if="!active.length && (!showFinished || !months.length)" class="fine subtle">{{ t('shelf.nothingHere') }}</p>
  </div>
</template>

<style scoped>
.shelf-section-heading { font-size: var(--text-xl); margin-bottom: var(--space-4); }
.finished-section.has-active { margin-top: var(--space-6); }
.shelf-month + .shelf-month { margin-top: var(--space-6); }
.month-heading { display: flex; align-items: baseline; gap: var(--space-3); font-family: var(--sans); font-weight: 400; letter-spacing: normal; color: var(--text-muted); font-size: var(--text-md); margin-bottom: var(--space-2); }
.month-heading .nums { font-size: var(--text-xs); }
.month-picker { margin-top: var(--space-5); }
.month-picker-form { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); }
.month-picker-form label { width: 100%; }
.month-picker-form input { flex: 1; min-width: 0; }
</style>
