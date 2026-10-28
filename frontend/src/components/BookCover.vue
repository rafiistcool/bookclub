<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { monogram, type CoverSize } from "../constants";
import { bookColorKey, COVER_RETRY_MS, coverSources, type CoverBook } from "../covers";
import { bindingColor, spineInk } from "../physicalShelf";

const { t } = useI18n();

const props = defineProps<{
  book: CoverBook;
  size?: CoverSize;
  eager?: boolean;
}>();

const size = computed<CoverSize>(() => props.size ?? "fluid");
// Only our own /media URLs: the browser never asks a third party for a cover.
const sources = computed(() => coverSources(props.book, size.value));
const sourceIndex = ref(0);
const retried = ref(false);
const waiting = ref(false);
// A new <img> element for the retry, so the browser asks the server again.
const attempt = ref(0);
let retryTimer: ReturnType<typeof setTimeout> | null = null;

const src = computed(() => (waiting.value ? null : (sources.value[sourceIndex.value] ?? null)));
// Same cloth colour as the book's spine on the bookshelf page.
const binding = computed(() => {
  const color = bindingColor(bookColorKey(props.book));
  return { "--binding": color, "--binding-ink": spineInk(color) };
});

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

function clearRetry() {
  if (retryTimer !== null) clearTimeout(retryTimer);
  retryTimer = null;
}

/** One retry after a short pause (the server may still be fetching the picture), then the next size, then the cloth. */
function fail() {
  if (!retried.value) {
    retried.value = true;
    waiting.value = true;
    retryTimer = setTimeout(() => {
      retryTimer = null;
      waiting.value = false;
      attempt.value += 1;
    }, COVER_RETRY_MS);
    return;
  }
  sourceIndex.value += 1;
  attempt.value += 1;
}

function onLoad(event: Event) {
  const img = event.target as HTMLImageElement;
  if (img.naturalWidth <= 1 || img.naturalHeight <= 1) fail();
}

watch(
  () => [props.book.cover_url, props.book.cover_thumb_url, size.value],
  () => {
    clearRetry();
    sourceIndex.value = 0;
    retried.value = false;
    waiting.value = false;
  },
);

onBeforeUnmount(clearRetry);
</script>

<template>
  <span class="cover" :class="`size-${size}`" :style="binding">
    <!-- A cloth binding sits underneath so a slow or missing cover still
         reads as a book instead of an empty rectangle. -->
    <span class="monogram" aria-hidden="true">{{ monogram(book.title) }}</span>
    <span class="binding-title" aria-hidden="true">{{ book.title }}</span>
    <img
      v-if="src"
      :key="attempt"
      :src="src"
      :alt="t('cover.alt', { title: book.title })"
      :width="width"
      :height="height"
      :loading="eager ? 'eager' : 'lazy'"
      decoding="async"
      @error="fail"
      @load="onLoad"
    />
  </span>
</template>
