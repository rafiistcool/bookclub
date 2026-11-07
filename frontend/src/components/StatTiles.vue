<script setup lang="ts">
import { computed } from "vue";

/** A grid of headline numbers: a figure each, with its label under it, split
    by hairlines like the numbers strip on the profile. */
const props = defineProps<{ items: readonly { key: string; label: string; value: string }[] }>();

/** Columns that fill every row: 3 for six figures, 2 for four. A wide screen
    sets four or fewer figures in one row. */
const columns = computed(() => {
  const n = props.items.length;
  const narrow = n % 3 === 0 ? 3 : n % 2 === 0 ? 2 : 3;
  return { "--cols": narrow, "--cols-wide": n <= 4 ? Math.max(n, 1) : narrow };
});
</script>

<template>
  <dl class="stat-tiles" :style="columns">
    <div v-for="item in items" :key="item.key" class="stat-tile" :data-stat="item.key">
      <dt>{{ item.label }}</dt>
      <dd class="nums">{{ item.value }}</dd>
    </div>
  </dl>
</template>

<style scoped>
.stat-tiles {
  display: grid;
  grid-template-columns: repeat(var(--cols, 3), minmax(0, 1fr));
  margin: 0;
  border-block: 1px solid var(--border);
  /* Clips the outer halves of the tiles' hairlines below. */
  overflow: hidden;
}

/* Each tile draws the hairline on its left and top; the first column's and
   the first row's fall outside the grid and are clipped, so only the lines
   between tiles show, whatever the column count. */
.stat-tile {
  display: flex;
  /* The label follows the figure in the markup (dt before dd); shown under it. */
  flex-direction: column-reverse;
  /* In column-reverse, flex-end is the top: every figure on one line even
     when a label wraps. */
  justify-content: flex-end;
  align-items: center;
  gap: 2px;
  min-width: 0;
  padding: var(--space-3) var(--space-2);
  text-align: center;
  box-shadow:
    -1px 0 0 var(--border),
    0 -1px 0 var(--border);
}

.stat-tile dd {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-2xl);
  font-weight: 600;
  line-height: var(--leading-tight);
  color: var(--text);
}

.stat-tile dt {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: 600;
  line-height: var(--leading-snug);
}

@media (min-width: 720px) {
  .stat-tiles {
    grid-template-columns: repeat(var(--cols-wide, 3), minmax(0, 1fr));
  }
}
</style>
