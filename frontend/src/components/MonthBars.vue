<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { MonthCount } from "../api/generated";
import { useLocale } from "../stores/locale";
import { monthName, monthSeries } from "../stats";

/**
 * Books finished per month of one year: one series, so no legend; every month
 * with a book carries its number (the busiest in full weight), and a table
 * beside it says the same for screen readers. The bars grow in once.
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
          v-for="(row, i) in rows"
          :key="row.month"
          class="bar-col"
          :class="{ peak: row.peak, zero: !row.finished }"
          :title="t('stats.monthFinished', { month: row.long, n: row.finished })"
        >
          <span class="bar" :style="{ height: row.height, '--i': i }">
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
  gap: var(--space-1);
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
  width: min(64%, 22px);
  background: var(--accent);
  border-radius: 3px 3px 0 0;
  transform-origin: bottom;
}

/* The page's one moment: the bars rise in turn, once. Without motion they
   simply stand there (no delay at scale 0 either). */
@media (prefers-reduced-motion: no-preference) {
  .bar {
    animation: bar-in var(--dur-slow) var(--ease-out) both;
    animation-delay: calc(var(--i, 0) * 30ms);
  }
}

@keyframes bar-in {
  from {
    transform: scaleY(0);
  }
}

.bar-value {
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%);
  font-size: var(--text-2xs);
  font-weight: 600;
  line-height: 1.1rem;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.bar-col.zero .bar-value {
  visibility: hidden;
}

.bar-col.peak .bar-value {
  font-weight: 700;
  color: var(--text);
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
