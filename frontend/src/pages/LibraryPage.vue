<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import NavIcon from "../components/NavIcon.vue";
import Sheet from "../components/Sheet.vue";
import Skeleton from "../components/Skeleton.vue";
import { SUBJECTS } from "../constants";
import { useFlow } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useShelf } from "../stores/shelf";
import type { SearchHit, SearchSort } from "../types";

const SORTS = [
  { label: "Popular", value: "readinglog" },
  { label: "New", value: "new" },
  { label: "Title", value: "title" },
] as const;

const flow = useFlow();
const shelf = useShelf();
const pickStore = usePick();

const query = ref("");
const submittedQuery = ref("");
const subject = ref("");
const sortPick = ref<"" | Exclude<SearchSort, "relevance">>("");
const hideOnShelf = ref(false);
const filtersOpen = ref(false);

const items = ref<SearchHit[]>([]);
const page = ref(0);
const hasMore = ref(true);
const pending = ref(true);
const error = ref("");
const sentinel = ref<HTMLElement | null>(null);

let requestSeq = 0;
let observer: IntersectionObserver | null = null;
let debounce = 0;

const effectiveSort = computed<SearchSort>(() => {
  if (sortPick.value) return sortPick.value;
  return submittedQuery.value ? "relevance" : "readinglog";
});

const clubKey = computed(() => pickStore.pick?.book.ol_work_key ?? null);

/** Overlay the live shelf so tiles update after add/move without a refetch. */
function statusOf(hit: SearchHit) {
  return shelf.loaded ? shelf.byKey.get(hit.ol_work_key)?.status ?? null : hit.on_shelf;
}

const visibleItems = computed(() =>
  hideOnShelf.value ? items.value.filter((hit) => !statusOf(hit)) : items.value,
);

const filterCount = computed(() => Number(Boolean(sortPick.value)) + Number(hideOnShelf.value));

async function loadPage(nextPage: number, reset: boolean) {
  if (!reset && (pending.value || !hasMore.value)) return;
  const seq = ++requestSeq;
  pending.value = true;
  error.value = "";
  if (reset) {
    items.value = [];
    page.value = 0;
    hasMore.value = true;
  }
  try {
    const result = await api.search({
      q: submittedQuery.value || undefined,
      subject: subject.value || undefined,
      sort: effectiveSort.value,
      page: nextPage,
    });
    if (seq !== requestSeq) return;
    const seen = new Set(items.value.map((hit) => hit.ol_work_key));
    for (const hit of result.items) {
      if (seen.has(hit.ol_work_key)) continue;
      items.value.push(hit);
      seen.add(hit.ol_work_key);
    }
    page.value = result.page;
    hasMore.value = result.has_more && result.items.length > 0;
  } catch (err) {
    if (seq !== requestSeq) return;
    error.value = err instanceof ApiError ? err.message : "Search failed";
    if (reset) items.value = [];
    hasMore.value = false;
  } finally {
    if (seq === requestSeq) {
      pending.value = false;
      queueMicrotask(maybeLoadMore);
    }
  }
}

function resetAndFetch() {
  void loadPage(1, true);
}

function maybeLoadMore() {
  if (pending.value || !hasMore.value) return;
  const el = sentinel.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  if (rect.top < window.innerHeight + 240) void loadPage(page.value + 1, false);
}

function onQueryInput() {
  window.clearTimeout(debounce);
  debounce = window.setTimeout(() => {
    const next = query.value.trim();
    if (next !== submittedQuery.value) {
      submittedQuery.value = next;
      resetAndFetch();
    }
  }, 350);
}

function submitSearch() {
  window.clearTimeout(debounce);
  submittedQuery.value = query.value.trim();
  resetAndFetch();
}

function clearSearch() {
  query.value = "";
  submittedQuery.value = "";
  resetAndFetch();
}

function toggleSubject(value: string) {
  subject.value = subject.value === value ? "" : value;
}

function pickSort(value: (typeof SORTS)[number]["value"]) {
  const implied: SearchSort = submittedQuery.value ? "relevance" : "readinglog";
  sortPick.value = value === implied || sortPick.value === value ? "" : value;
}

function openHit(hit: SearchHit) {
  flow.open({
    kind: "details",
    book: {
      ol_work_key: hit.ol_work_key,
      title: hit.title,
      authors: hit.authors,
      cover_id: hit.cover_id,
      year: hit.year,
    },
  });
}

watch([subject, sortPick], resetAndFetch);
watch(sentinel, (el, prev) => {
  if (!observer) return;
  if (prev) observer.unobserve(prev);
  if (el) observer.observe(el);
});

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) maybeLoadMore();
    },
    { rootMargin: "240px 0px" },
  );
  if (sentinel.value) observer.observe(sentinel.value);
  void shelf.load();
  void pickStore.load();
  resetAndFetch();
});

onUnmounted(() => {
  observer?.disconnect();
  observer = null;
  requestSeq += 1;
  window.clearTimeout(debounce);
});
</script>

<template>
  <section aria-label="Library">
    <div class="library-head">
      <form class="search-row" role="search" @submit.prevent="submitSearch">
        <div class="search-box">
          <NavIcon name="search" :size="18" />
          <input
            v-model="query"
            type="search"
            inputmode="search"
            enterkeyhint="search"
            placeholder="Title or author"
            aria-label="Search books"
            autocomplete="off"
            @input="onQueryInput"
          />
          <button v-if="query" class="icon-btn clear" type="button" aria-label="Clear search" @click="clearSearch">
            <NavIcon name="close" :size="16" />
          </button>
        </div>
        <button class="icon-btn" type="button" aria-label="Scan a barcode" @click="flow.open({ kind: 'scan' })">
          <NavIcon name="camera" />
        </button>
        <button
          class="icon-btn"
          type="button"
          aria-label="Sort and filters"
          :aria-pressed="filtersOpen"
          @click="filtersOpen = true"
        >
          <NavIcon name="filter" />
          <span v-if="filterCount" class="dot" aria-hidden="true" />
        </button>
      </form>
      <div class="chip-row" role="group" aria-label="Subject" style="margin-top: 8px">
        <button
          v-for="chip in SUBJECTS"
          :key="chip.value"
          class="chip"
          type="button"
          :aria-pressed="subject === chip.value"
          :class="{ active: subject === chip.value }"
          @click="toggleSubject(chip.value)"
        >
          {{ chip.label }}
        </button>
      </div>
    </div>

    <p v-if="error" class="error" style="margin-bottom: 12px">
      {{ error }}
      <button class="text-btn sm" type="button" @click="resetAndFetch">Retry</button>
    </p>

    <div v-if="visibleItems.length" class="library-grid">
      <BookTile
        v-for="hit in visibleItems"
        :key="hit.ol_work_key"
        :title="hit.title"
        :authors="hit.authors"
        :cover-id="hit.cover_id"
        :status="statusOf(hit)"
        :club-pick="hit.ol_work_key === clubKey"
        @open="openHit(hit)"
      />
    </div>
    <Skeleton v-else-if="pending" kind="tiles" :count="12" />
    <div v-else-if="items.length > 0 && hideOnShelf" class="empty">
      <p>Every loaded book is already on your shelf.</p>
      <button class="btn btn-ghost btn-sm" type="button" @click="hideOnShelf = false">Show them</button>
    </div>
    <div v-else-if="!error" class="empty">
      <p>No books matched that.</p>
      <p class="fine">Try a shorter title, another subject, or <button class="text-btn sm" type="button" @click="flow.open({ kind: 'scan' })">scan the barcode</button>.</p>
    </div>

    <div ref="sentinel" class="library-sentinel" aria-hidden="true" />
    <div v-if="pending && items.length > 0" class="library-more"><span class="spinner" /></div>

    <Sheet v-if="filtersOpen" title="Sort and filters" @close="filtersOpen = false">
      <div class="field">
        <span class="field-label">Sort</span>
        <div class="chip-row" role="group" aria-label="Sort">
          <button
            v-for="option in SORTS"
            :key="option.value"
            class="chip"
            type="button"
            :aria-pressed="effectiveSort === option.value"
            :class="{ active: effectiveSort === option.value }"
            @click="pickSort(option.value)"
          >
            {{ option.label }}
          </button>
          <span v-if="submittedQuery && effectiveSort === 'relevance'" class="chip active">Best match</span>
        </div>
      </div>
      <div class="toggle-row">
        <span>Hide books already on my shelf</span>
        <button class="switch" type="button" role="switch" :aria-checked="hideOnShelf" @click="hideOnShelf = !hideOnShelf" />
      </div>
      <template #foot>
        <button class="btn btn-primary" type="button" @click="filtersOpen = false">Done</button>
      </template>
    </Sheet>
  </section>
</template>
