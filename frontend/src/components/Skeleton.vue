<script setup lang="ts">
/** Flat placeholder blocks. `kind` picks a preset; `count` repeats it. */
defineProps<{
  kind: "cards" | "tiles" | "lines" | "hero" | "feed";
  count?: number;
}>();
</script>

<template>
  <div aria-busy="true" aria-live="polite">
    <template v-if="kind === 'cards'">
      <div class="list">
        <div v-for="n in count ?? 3" :key="n" class="book-card" style="border-color: transparent">
          <div class="skeleton cover" style="width: 56px" />
          <div class="meta" style="gap: 8px">
            <div class="skeleton line" style="width: 70%" />
            <div class="skeleton line" style="width: 45%" />
            <div class="skeleton line" style="width: 30%; height: 0.7em" />
          </div>
        </div>
      </div>
    </template>
    <template v-else-if="kind === 'tiles'">
      <div class="library-grid">
        <div v-for="n in count ?? 9" :key="n" class="book-tile">
          <div class="skeleton cover" style="width: 100%" />
          <div class="skeleton line" style="width: 80%" />
        </div>
      </div>
    </template>
    <template v-else-if="kind === 'hero'">
      <div class="pick-hero">
        <div class="skeleton cover" style="width: 96px" />
        <div class="meta" style="gap: 10px">
          <div class="skeleton line" style="width: 40%; height: 0.7em" />
          <div class="skeleton line" style="width: 85%; height: 1.3em" />
          <div class="skeleton line" style="width: 55%" />
          <div class="skeleton line" style="width: 65%" />
        </div>
      </div>
    </template>
    <template v-else-if="kind === 'feed'">
      <div class="feed">
        <div v-for="n in count ?? 5" :key="n" class="feed-item" style="border-color: transparent">
          <div class="skeleton circle" style="width: 32px; height: 32px" />
          <div style="display: grid; gap: 6px">
            <div class="skeleton line" style="width: 80%" />
            <div class="skeleton line" style="width: 30%; height: 0.7em" />
          </div>
        </div>
      </div>
    </template>
    <template v-else>
      <div style="display: grid; gap: 8px">
        <div v-for="n in count ?? 3" :key="n" class="skeleton line" :style="{ width: `${90 - n * 12}%` }" />
      </div>
    </template>
  </div>
</template>
