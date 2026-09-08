<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import type { SearchHit, SearchSort } from "../types";

const SUBJECTS = [
  { label: "Fiction", value: "fiction" },
  { label: "Fantasy", value: "fantasy" },
  { label: "Mystery", value: "mystery" },
  { label: "Romance", value: "romance" },
  { label: "Science fiction", value: "science_fiction" },
  { label: "History", value: "history" },
  { label: "Biography", value: "biography" },
  { label: "Horror", value: "horror" },
  { label: "Young adult", value: "young_adult" },
] as const;

/** The shelves on the browse surface, in the order they appear. */
const BROWSE_ROWS = [
  "fiction",
  "science_fiction",
  "mystery",
  "history",
  "fantasy",
] as const;

const SORTS = [
  { label: "Popular", value: "readinglog" },
  { label: "New", value: "new" },
  { label: "Title", value: "title" },
] as const;

type Row = {
  subject: string;
  label: string;
  items: SearchHit[];
  pending: boolean;
};

const route = useRoute();
const router = useRouter();

const query = ref("");
const submittedQuery = ref("");
const subject = ref("");
const sortPick = ref<"" | Exclude<SearchSort, "relevance">>("");
const hideOnShelf = ref(false);

const items = ref<SearchHit[]>([]);
const page = ref(0);
const hasMore = ref(true);
const pending = ref(false);
const error = ref("");
const sentinel = ref<HTMLElement | null>(null);

const trending = ref<SearchHit[]>([]);
const rows = ref<Row[]>([]);
const browsePending = ref(true);
const browseError = ref("");

let requestSeq = 0;
let observer: IntersectionObserver | null = null;

/** Browse is the default surface; a query or subject switches to results. */
const browsing = computed(() => !submittedQuery.value && !subject.value);

const effectiveSort = computed<SearchSort>(() => {
  if (sortPick.value) return sortPick.value;
  return submittedQuery.value ? "relevance" : "readinglog";
});

const visibleItems = computed(() =>
  hideOnShelf.value ? items.value.filter((hit) => !hit.on_shelf) : items.value,
);

const resultsLabel = computed(() => {
  const chip = SUBJECTS.find((entry) => entry.value === subject.value);
  if (submittedQuery.value && chip) return `“${submittedQuery.value}” in ${chip.label}`;
  if (submittedQuery.value) return `“${submittedQuery.value}”`;
  return chip?.label ?? "Results";
});

function subjectLabel(value: string) {
  return (
    SUBJECTS.find((entry) => entry.value === value)?.label ??
    value.replace(/_/g, " ")
  );
}

/**
 * Trending and every subject shelf load independently so that one upstream
 * failure leaves the rest of the browse surface usable.
 */
async function loadBrowse() {
  browsePending.value = true;
  browseError.value = "";
  rows.value = BROWSE_ROWS.map((value) => ({
    subject: value,
    label: subjectLabel(value),
    items: [],
    pending: true,
  }));
  await Promise.all([
    (async () => {
      try {
        trending.value = (await api.trending(14)).items;
      } catch (err) {
        trending.value = [];
        browseError.value =
          err instanceof ApiError ? err.message : "Could not reach Open Library";
      } finally {
        browsePending.value = false;
      }
    })(),
    // Open Library throttles bursts, so the shelves queue up behind each other
    // instead of firing all at once. Each one reveals itself as it lands.
    (async () => {
      for (const row of rows.value) {
        try {
          row.items = (await api.subject(row.subject, 1, 14)).items;
        } catch {
          row.items = [];
        } finally {
          row.pending = false;
        }
      }
    })(),
  ]);
}

function retryTrending() {
  browsePending.value = true;
  browseError.value = "";
  void api
    .trending(14)
    .then((result) => {
      trending.value = result.items;
    })
    .catch((err) => {
      browseError.value =
        err instanceof ApiError ? err.message : "Could not reach Open Library";
    })
    .finally(() => {
      browsePending.value = false;
    });
}

async function loadPage(nextPage: number, reset: boolean) {
  if (!reset && (pending.value || !hasMore.value)) return;
  const seq = ++requestSeq;
  pending.value = true;
  error.value = "";
  if (reset) {
    // Keep the current tiles on screen until this request lands. Clearing
    // here flashes an empty grid, which is obvious on a cached refetch.
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
    if (reset) {
      items.value = result.items;
    } else {
      const seen = new Set(items.value.map((hit) => hit.ol_work_key));
      for (const hit of result.items) {
        if (seen.has(hit.ol_work_key)) continue;
        items.value.push(hit);
        seen.add(hit.ol_work_key);
      }
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

function maybeLoadMore() {
  if (browsing.value || pending.value || !hasMore.value) return;
  const el = sentinel.value;
  if (!el) return;
  if (el.getBoundingClientRect().top < window.innerHeight + 240) {
    void loadPage(page.value + 1, false);
  }
}

function syncRoute() {
  const next: Record<string, string> = {};
  if (submittedQuery.value) next.q = submittedQuery.value;
  if (subject.value) next.subject = subject.value;
  void router.replace({ path: "/discover", query: next });
}

function submitSearch() {
  submittedQuery.value = query.value.trim();
  syncRoute();
}

function clearSearch() {
  query.value = "";
  submittedQuery.value = "";
  syncRoute();
}

function toggleSubject(value: string) {
  subject.value = subject.value === value ? "" : value;
  syncRoute();
}

function pickSort(value: (typeof SORTS)[number]["value"]) {
  const implied: SearchSort = submittedQuery.value ? "relevance" : "readinglog";
  sortPick.value = sortPick.value === value || value === implied ? "" : value;
}

/** Returns true when it changed state, which means the watcher will fetch. */
function readRoute(): boolean {
  const q = typeof route.query.q === "string" ? route.query.q : "";
  const s = typeof route.query.subject === "string" ? route.query.subject : "";
  if (q === submittedQuery.value && s === subject.value) return false;
  query.value = q;
  submittedQuery.value = q;
  subject.value = s;
  return true;
}

watch(
  () => [route.query.q, route.query.subject],
  () => readRoute(),
);

watch([submittedQuery, subject, sortPick], () => {
  if (browsing.value) {
    requestSeq += 1;
    hasMore.value = true;
    pending.value = false;
    error.value = "";
    return;
  }
  void loadPage(1, true);
});

watch(sentinel, (el, previous) => {
  if (!observer) return;
  if (previous) observer.unobserve(previous);
  if (el) observer.observe(el);
});

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) maybeLoadMore();
    },
    { rootMargin: "240px 0px" },
  );
  const willFetch = readRoute();
  void loadBrowse();
  if (!willFetch && !browsing.value) void loadPage(1, true);
});

onUnmounted(() => {
  observer?.disconnect();
  observer = null;
  requestSeq += 1;
});
</script>

<template>
  <section>
    <div class="page-head">
      <h1>Discover</h1>
      <p class="lede">Search the Open Library, or browse what people are reading.</p>
    </div>

    <div class="discover-search">
      <form role="search" @submit.prevent="submitSearch">
        <span class="search-field">
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="6.4" />
            <path d="M15.8 15.8 20.2 20.2" />
          </svg>
          <input
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
        </span>
        <button class="btn btn-primary" type="submit">Search</button>
      </form>
      <div class="chip-row scroll" role="group" aria-label="Subject">
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

    <!-- Browse surface -->
    <template v-if="browsing">
      <section aria-labelledby="trending">
        <div class="section-head">
          <h2 id="trending">Trending today</h2>
        </div>
        <div v-if="browsePending || trending.length" class="rail">
          <TileSkeleton v-if="browsePending" :count="7" />
          <BookTile
            v-for="hit in trending"
            v-else
            :key="hit.ol_work_key"
            :ol-work-key="hit.ol_work_key"
            :title="hit.title"
            :authors="hit.authors"
            :cover-id="hit.cover_id"
            :status="hit.on_shelf"
            :club-pick="hit.club_pick"
            show-authors
          />
        </div>
        <p v-else-if="browseError" class="fine subtle row-fallback">
          {{ browseError }}
          <button class="text-btn" type="button" @click="retryTrending">Try again</button>
        </p>
        <p v-else class="fine subtle">Nothing trending right now.</p>
      </section>

      <section v-for="row in rows" :key="row.subject" class="section">
        <div class="section-head">
          <h2>{{ row.label }}</h2>
          <button class="text-btn" type="button" @click="toggleSubject(row.subject)">
            See all
          </button>
        </div>
        <div v-if="row.pending || row.items.length" class="rail">
          <TileSkeleton v-if="row.pending" :count="7" />
          <BookTile
            v-for="hit in row.items"
            v-else
            :key="hit.ol_work_key"
            :ol-work-key="hit.ol_work_key"
            :title="hit.title"
            :authors="hit.authors"
            :cover-id="hit.cover_id"
            :status="hit.on_shelf"
            :club-pick="hit.club_pick"
            show-authors
          />
        </div>
        <p v-else class="fine subtle row-fallback">
          Could not load this shelf.
          <button class="text-btn" type="button" @click="toggleSubject(row.subject)">
            Search it instead
          </button>
        </p>
      </section>
    </template>

    <!-- Results surface -->
    <template v-else>
      <div class="results-head">
        <h2>{{ resultsLabel }}</h2>
        <div class="chip-row scroll">
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
          <button
            class="chip"
            type="button"
            :aria-pressed="hideOnShelf"
            :class="{ active: hideOnShelf }"
            @click="hideOnShelf = !hideOnShelf"
          >
            Not on my shelf
          </button>
        </div>
      </div>

      <div v-if="error" class="empty">
        <h3>That search did not come back</h3>
        <p>{{ error }}</p>
        <div class="btn-row">
          <button class="btn btn-ghost" type="button" @click="loadPage(1, true)">
            Try again
          </button>
          <button class="btn btn-ghost" type="button" @click="clearSearch">
            Back to browsing
          </button>
        </div>
      </div>

      <div v-else-if="visibleItems.length || pending" class="book-grid">
        <BookTile
          v-for="hit in visibleItems"
          :key="hit.ol_work_key"
          :ol-work-key="hit.ol_work_key"
          :title="hit.title"
          :authors="hit.authors"
          :cover-id="hit.cover_id"
          :status="hit.on_shelf"
          :club-pick="hit.club_pick"
          show-authors
        />
        <TileSkeleton v-if="pending" :count="visibleItems.length ? 6 : 12" />
      </div>

      <div v-else-if="items.length && hideOnShelf" class="empty">
        <h3>All of these are already yours</h3>
        <p>Every loaded result is on your shelf.</p>
        <div class="btn-row">
          <button class="btn btn-ghost" type="button" @click="hideOnShelf = false">
            Show them anyway
          </button>
        </div>
      </div>

      <div v-else class="empty">
        <h3>Nothing matched</h3>
        <p>Try a shorter title, an author's name, or another subject.</p>
        <div class="btn-row">
          <button class="btn btn-ghost" type="button" @click="clearSearch">
            Back to browsing
          </button>
        </div>
      </div>

      <div ref="sentinel" class="sentinel" aria-hidden="true" />
    </template>
  </section>
</template>

<style scoped>
.discover-search {
  position: sticky;
  top: calc(var(--header-h) + env(safe-area-inset-top));
  z-index: 15;
  margin: calc(-1 * var(--space-2)) calc(-1 * var(--space-4)) var(--space-5);
  padding: var(--space-2) var(--space-4) var(--space-3);
  background: var(--bg-translucent-strong);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--border);
}

.discover-search form {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
}

.search-field {
  flex: 1;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 0 var(--space-3);
  min-height: 46px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--text-subtle);
  min-width: 0;
}

.search-field input {
  flex: 1;
  min-width: 0;
  border: 0;
  background: transparent;
  padding: 0;
  color: var(--text);
}

.search-field input:focus-visible {
  outline: none;
}

.search-field:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}

.results-head {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

.results-head h2 {
  font-size: var(--text-lg);
}

.row-fallback {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
}

.sentinel {
  height: 1px;
}

@media (min-width: 1024px) {
  .discover-search {
    top: 0;
    margin-top: calc(-1 * var(--space-7));
    margin-left: calc(-1 * var(--space-7));
    margin-right: calc(-1 * var(--space-7));
    padding: var(--space-5) var(--space-7) var(--space-3);
  }

  .discover-search form {
    max-width: 640px;
  }

  .results-head {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
}
</style>
