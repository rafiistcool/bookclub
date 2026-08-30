<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { coverUrl, monogram, type CoverSize } from "../constants";

const props = defineProps<{
  title: string;
  coverId?: number | null;
  size?: CoverSize;
}>();

const broken = ref(false);
const size = computed<CoverSize>(() => props.size ?? "fluid");
const src = computed(() => coverUrl(props.coverId, size.value));

// Intrinsic dimensions at a 2:3 ratio, so the row does not reflow when the
// image arrives. Fluid covers take their width from the grid track instead.
const WIDTHS: Record<Exclude<CoverSize, "fluid">, number> = {
  xs: 36,
  sm: 52,
  md: 72,
  lg: 104,
};
const width = computed(() =>
  size.value === "fluid" ? undefined : WIDTHS[size.value],
);
const height = computed(() => (width.value ? Math.round(width.value * 1.5) : undefined));

watch(
  () => props.coverId,
  () => {
    broken.value = false;
  },
);
</script>

<template>
  <span class="cover" :class="`size-${size}`">
    <!-- The monogram sits underneath so a slow or missing cover still reads
         as a book instead of an empty rectangle. -->
    <span class="monogram" aria-hidden="true">{{ monogram(title) }}</span>
    <img
      v-if="src && !broken"
      :src="src"
      :alt="`Cover of ${title}`"
      :width="width"
      :height="height"
      loading="lazy"
      decoding="async"
      @error="broken = true"
    />
  </span>
</template>
