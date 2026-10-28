<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { MonthCount } from "../api/generated";
import { useLocale } from "../stores/locale";
import { monthName, monthSeries } from "../stats";

/**
 * Books finished per month of one year: one series, so no legend; the busiest
 * month carries its number, every bar shows its own on hover or focus, and a
 * table beside it says the same for screen readers.
 */
const props = defineProps<{ year: number; months: readonly MonthCount[] }>();

const { t } = useI18n();
const locale = useLocale();

const series = computed(() => monthSeries(props.year, props.months));
const most = computed(() => Math.max(0, ...series.value.map((row) => row.finished)));
const rows = computed(() =>
  series.value.map((row) => ({
    ...row,
    short: monthName(row.month, locale.locale),
    long: monthName(row.month, locale.locale, "long"),
    height: most.value ? `${(row.finished / most.value) * 100}%` : "0%",
    peak: row.finished > 0 && row.finished === most.value,
  })),
);
</script>

<template>
  <figure class="month-bars">
    <figcaption class="section-head">
      <h3>{{ t("stats.byMonth") }}</h3>
    </figcaption>
    <div class="plot" aria-hidden="true">
      <div class="bars">
        <span
          v-for="row in rows"
          :key="row.month"
          class="bar-col"
          :class="{ peak: row.peak, zero: !row.finished }"
          :title="t('stats.monthFinished', { month: row.long, n: row.finished })"
        >
          <span class="bar" :style="{ height: row.height }">
            <span class="bar-value">{{ row.finished }}</span>
          </span>
        </span>
      </div>
      <div class="labels">
        <span v-for="row in rows" :key="row.month">{{ row.short }}</span>
      </div>
    </div>
    <table class="visually-hidden">
      <caption>{{ t("stats.byMonthCaption", { year }) }}</caption>
      <thead>
        <tr>
          <th scope="col">{{ t("stats.month") }}</th>
          <th scope="col">{{ t("stats.finished") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.month">
          <th scope="row">{{ row.long }}</th>
          <td>{{ row.finished }}</td>
        </tr>
      </tbody>
    </table>
  </figure>
</template>

<style scoped>
/* The caller spaces it from what comes before (`class="section"`). */
.month-bars {
  margin-inline: 0;
  margin-bottom: 0;
}

.bars,
.labels {
  display: grid;
  grid-template-columns: repeat(12, minmax(0, 1fr));
  gap: 2px;
}

.bars {
  height: 140px;
  align-items: stretch;
  /* The baseline: recessive, right under the bars. */
  border-bottom: 1px solid var(--border);
}

.bar-col {
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  align-items: center;
  min-width: 0;
  /* Room for the value above the tallest bar. */
  padding-top: 1.25rem;
}

.bar {
  position: relative;
  display: block;
  width: min(100%, 28px);
  background: var(--accent);
  border-radius: 4px 4px 0 0;
}

.bar-value {
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%);
  font-size: var(--text-2xs);
  font-weight: 700;
  line-height: 1.1rem;
  color: var(--text-muted);
  visibility: hidden;
}

.bar-col.peak .bar-value,
.bar-col:not(.zero):hover .bar-value {
  visibility: visible;
}

.labels span {
  text-align: center;
  font-size: var(--text-2xs);
  color: var(--text-muted);
  padding-top: var(--space-1);
  overflow: hidden;
  white-space: nowrap;
}
</style>
