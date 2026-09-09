<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  coverUrl,
  editionCoverUrl,
  isbnCoverUrl,
  isbnFromWorkKey,
  isGoogleCatalogWorkKey,
  monogram,
  remoteCoverUrl,
  type CoverSize,
} from "../constants";

const props = defineProps<{
  title: string;
  coverId?: number | null;
  coverEditionKey?: string | null;
  isbn?: string | null;
  workKey?: string | null;
  imageUrl?: string | null;
  size?: CoverSize;
  eager?: boolean;
}>();

const broken = ref(false);
const sourceIndex = ref(0);
const size = computed<CoverSize>(() => props.size ?? "fluid");
const sources = computed(() => {
  const remote = remoteCoverUrl(props.imageUrl);
  if (remote) return [remote];
  const urls = [coverUrl(props.coverId, size.value)];
  // ISBN / GB keys store a Google URL on the book. Do not invent an
  // Open Library ISBN CDN request for those rows.
  if (!isGoogleCatalogWorkKey(props.workKey || "")) {
    urls.push(
      editionCoverUrl(props.coverEditionKey, size.value),
      isbnCoverUrl(props.isbn || isbnFromWorkKey(props.workKey || ""), size.value),
    );
  }
  return urls.filter((url): url is string => Boolean(url));
});
const src = computed(() => (broken.value ? null : (sources.value[sourceIndex.value] ?? null)));

// Intrinsic dimensions at a 2:3 ratio, so the row does not reflow when the
// image arrives. Fluid / tile covers take their width from the grid track.
const WIDTHS: Record<Exclude<CoverSize, "fluid" | "tile">, number> = {
  xs: 36,
  sm: 52,
  md: 72,
  lg: 104,
};
const width = computed(() =>
  size.value === "fluid" || size.value === "tile" ? undefined : WIDTHS[size.value],
);
const height = computed(() => (width.value ? Math.round(width.value * 1.5) : undefined));

function fail() {
  if (sourceIndex.value + 1 < sources.value.length) {
    sourceIndex.value += 1;
    return;
  }
  broken.value = true;
}

function onLoad(event: Event) {
  const img = event.target as HTMLImageElement;
  if (img.naturalWidth <= 1 || img.naturalHeight <= 1) {
    fail();
  }
}

watch(
  () => [
    props.imageUrl,
    props.coverId,
    props.coverEditionKey,
    props.isbn,
    props.workKey,
    size.value,
  ],
  () => {
    broken.value = false;
    sourceIndex.value = 0;
  },
);
</script>

<template>
  <span class="cover" :class="`size-${size}`">
    <!-- The monogram sits underneath so a slow or missing cover still reads
         as a book instead of an empty rectangle. -->
    <span class="monogram" aria-hidden="true">{{ monogram(title) }}</span>
    <img
      v-if="src"
      :src="src"
      :alt="`Cover of ${title}`"
      :width="width"
      :height="height"
      :loading="eager ? 'eager' : 'lazy'"
      decoding="async"
      @error="fail"
      @load="onLoad"
    />
  </span>
</template>
