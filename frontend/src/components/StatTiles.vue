<script setup lang="ts">
/** A row of headline numbers: a figure each, with its label under it. */
defineProps<{ items: readonly { key: string; label: string; value: string }[] }>();
</script>

<template>
  <dl class="stat-tiles">
    <div v-for="item in items" :key="item.key" class="stat-tile" :data-stat="item.key">
      <dt class="finer subtle">{{ item.label }}</dt>
      <dd>{{ item.value }}</dd>
    </div>
  </dl>
</template>

<style scoped>
.stat-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(8.5rem, 1fr));
  gap: var(--space-2);
  margin: 0;
}

.stat-tile {
  display: flex;
  flex-direction: column-reverse;
  gap: 2px;
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.stat-tile dd {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-xl);
  font-weight: 700;
  line-height: 1.1;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.stat-tile dt {
  margin: 0;
}
</style>
