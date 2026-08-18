<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import StatusSheet from "../components/StatusSheet.vue";
import type { Status } from "../constants";
import { STATUS_LABEL } from "../constants";
import { useToast } from "../stores/toast";
import type { SearchHit, SearchSort, ShelfItem } from "../types";

const SUBJECTS = [
  { label: "Fiction", value: "fiction" },
  { label: "Fantasy", value: "fantasy" },
  { label: "Mystery", value: "mystery" },
  { label: "Romance", value: "romance" },
  { label: "Sci-Fi", value: "science_fiction" },
  { label: "History", value: "history" },
  { label: "Biography", value: "biography" },
  { label: "Horror", value: "horror" },
  { label: "YA", value: "young_adult" },
] as const;

const SORTS = [
  { label: "Popular", value: "readinglog" },
  { label: "New", value: "new" },
  { label: "Title", value: "title" },
] as const;

const query = ref("");
const submittedQuery = ref("");
const subject = ref("");
const sortPick = ref<"" | Exclude<SearchSort, "relevance">>("");
const hideOnShelf = ref(false);
const searchOpen = ref(false);
const filtersOpen = ref(false);
const searchInput = ref<HTMLInputElement | null>(null);

const items = ref<SearchHit[]>([]);
const page = ref(0);
const hasMore = ref(true);
const pending = ref(true);
const error = ref("");
const toast = useToast();

const adding = ref<SearchHit | null>(null);
const moving = ref<SearchHit | null>(null);
const existing = ref<ShelfItem | null>(null);
const sentinel = ref<HTMLElement | null>(null);

let requestSeq = 0;
let observer: IntersectionObserver | null = null;

const effectiveSort = computed<SearchSort>(() => {
  if (sortPick.value) return sortPick.value;
  return submittedQuery.value ? "relevance" : "readinglog";
});

const visibleItems = computed(() =>
  hideOnShelf.value ? items.value.filter((hit) => !hit.on_shelf) : items.value,
);

const filtersActive = computed(
  () => Boolean(subject.value || sortPick.value || hideOnShelf.value),
);

function openBook(hit: SearchHit) {
  if (hit.on_shelf) {
    moving.value = hit;
    return;
  }
  adding.value = hit;
}

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
  if (rect.top < window.innerHeight + 240) {
    void loadPage(page.value + 1, false);
  }
}

function submitSearch() {
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
  if (sortPick.value === value) {
    sortPick.value = "";
    return;
  }
  const implied: SearchSort = submittedQuery.value ? "relevance" : "readinglog";
  sortPick.value = value === implied ? "" : value;
}

async function toggleSearch() {
  searchOpen.value = !searchOpen.value;
  if (searchOpen.value) {
    await nextTick();
    searchInput.value?.focus();
  }
}

async function add(status: Status) {
  const hit = adding.value;
  adding.value = null;
  if (!hit) return;
  try {
    const created = await api.addToShelf({
      ol_work_key: hit.ol_work_key,
      title: hit.title,
      authors: hit.authors,
      cover_id: hit.cover_id,
      year: hit.year,
      status,
    });
    hit.on_shelf = status;
    hit.shelf_id = created.id;
    toast.show(`Added to ${STATUS_LABEL[status]}`);
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      existing.value = err.item;
      moving.value = hit;
      return;
    }
    toast.show(err instanceof ApiError ? err.message : "Could not add that book");
  }
}

async function moveExisting(status: Status) {
  const item = existing.value;
  const hit = moving.value;
  const id = item?.id ?? hit?.shelf_id;
  moving.value = null;
  existing.value = null;
  if (!id) return;
  try {
    await api.patchShelf(id, { status, position: 0 });
    if (hit) {
      hit.on_shelf = status;
      hit.shelf_id = id;
    }
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not move that book");
  }
}

watch([subject, sortPick], () => {
  resetAndFetch();
});

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
  resetAndFetch();
});

onUnmounted(() => {
  observer?.disconnect();
  observer = null;
  requestSeq += 1;
});
</script>

<template>
  <section class="library-page">
    <h1 class="visually-hidden">Library</h1>
    <div class="library-toolbar">
      <div class="library-toolbar-row">
        <button
          class="icon-btn"
          type="button"
          :aria-pressed="searchOpen"
          :aria-expanded="searchOpen"
          aria-label="Search"
          @click="toggleSearch"
        >
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <circle cx="11" cy="11" r="6.5" stroke="currentColor" stroke-width="1.8" />
            <path d="M16 16.5 20 20.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
          <span v-if="submittedQuery" class="toolbar-dot" aria-hidden="true" />
        </button>
        <button
          class="icon-btn"
          type="button"
          :aria-pressed="filtersOpen"
          :aria-expanded="filtersOpen"
          aria-label="Filters"
          @click="filtersOpen = !filtersOpen"
        >
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path d="M4 7h16M7 12h10M10 17h4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
          <span v-if="filtersActive" class="toolbar-dot" aria-hidden="true" />
        </button>
      </div>
      <form v-if="searchOpen" class="library-search" @submit.prevent="submitSearch">
        <input
          ref="searchInput"
          v-model="query"
          type="search"
          inputmode="search"
          placeholder="Title or author"
          aria-label="Search books"
        />
        <button
          v-if="query || submittedQuery"
          class="text-btn"
          type="button"
          @click="clearSearch"
        >
          Clear
        </button>
      </form>
      <div v-if="filtersOpen" class="library-filters">
        <div class="chip-row" role="group" aria-label="Subject">
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
        </div>
        <div class="chip-row">
          <button
            class="chip"
            type="button"
            :aria-pressed="hideOnShelf"
            :class="{ active: hideOnShelf }"
            @click="hideOnShelf = !hideOnShelf"
          >
            Hide books already on my shelf
          </button>
        </div>
      </div>
    </div>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-if="visibleItems.length" class="library-grid">
      <BookTile
        v-for="hit in visibleItems"
        :key="hit.ol_work_key"
        :hit="hit"
        @open="openBook(hit)"
      />
    </div>
    <div v-else-if="pending" class="empty">Loading…</div>
    <div v-else-if="items.length > 0 && hideOnShelf" class="empty">
      Every loaded book is already on your shelf.
    </div>
    <div v-else-if="!error" class="empty">
      No books matched that. Try a shorter title or another subject.
    </div>
    <div ref="sentinel" class="library-sentinel" aria-hidden="true" />
    <div v-if="pending && items.length > 0" class="library-more">
      <span class="spinner" />
    </div>
    <StatusSheet
      v-if="adding"
      title="Add to shelf"
      current="want_to_read"
      @pick="add"
      @close="adding = null"
    />
    <StatusSheet
      v-if="moving"
      :title="existing ? 'Already on your shelf — move it?' : 'Move to…'"
      :current="existing?.status ?? moving.on_shelf"
      @pick="moveExisting"
      @close="moving = null; existing = null"
    />
  </section>
</template>
