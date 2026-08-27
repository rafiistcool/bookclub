<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import ClubPickBanner from "../components/ClubPickBanner.vue";
import ClubPickSheet from "../components/ClubPickSheet.vue";
import FinishNoteSheet from "../components/FinishNoteSheet.vue";
import StatusSheet from "../components/StatusSheet.vue";
import type { FinishNote, Status } from "../constants";
import { STATUS_LABEL } from "../constants";
import { useToast } from "../stores/toast";
import type { ClubPick, SearchHit, SearchSort, ShelfItem } from "../types";

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

const router = useRouter();
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
const clubPick = ref<ClubPick | null>(null);
const clubTimezone = ref("UTC");
const clubAdding = ref(false);
const clubMoving = ref(false);
const settingPick = ref<SearchHit | ClubPick["book"] | null>(null);
const finishing = ref<{
  mode: "add" | "move" | "club-add" | "club-move";
  status: "finished" | "did_not_finish";
  title: string;
  hit?: SearchHit | null;
  item?: ShelfItem | null;
} | null>(null);

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
    if (clubPick.value) markClubPick(clubPick.value.book.ol_work_key);
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

async function loadClubPick() {
  try {
    const current = await api.clubPick();
    clubPick.value = current.pick;
    clubTimezone.value = current.timezone;
  } catch {
    clubPick.value = null;
  }
}

function markClubPick(key: string) {
  for (const hit of items.value) {
    hit.club_pick = hit.ol_work_key === key;
  }
}

function startClubPick(hit: SearchHit | ClubPick["book"] | null) {
  adding.value = null;
  moving.value = null;
  existing.value = null;
  if (!hit) return;
  settingPick.value = hit;
}

async function nominate(hit: SearchHit | null) {
  adding.value = null;
  moving.value = null;
  existing.value = null;
  if (!hit) return;
  try {
    await api.nominate({
      ol_work_key: hit.ol_work_key,
      title: hit.title,
      authors: hit.authors,
      cover_id: hit.cover_id,
      year: hit.year,
    });
    toast.show("Nominated for next up");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
  }
}

async function confirmClubPick(meetingAt: string | null) {
  const hit = settingPick.value;
  settingPick.value = null;
  if (!hit) return;
  try {
    const next = await api.setClubPick({
      ol_work_key: hit.ol_work_key,
      title: hit.title,
      authors: hit.authors,
      cover_id: hit.cover_id,
      year: hit.year,
      meeting_at: meetingAt,
    });
    clubPick.value = next;
    markClubPick(next.book.ol_work_key);
    toast.show("Set as the club pick");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the club pick");
  }
}

async function addClubPick(status: Status, note: FinishNote = {}) {
  const current = clubPick.value;
  clubAdding.value = false;
  if (!current) return;
  try {
    const created = await api.addToShelf({
      ol_work_key: current.book.ol_work_key,
      title: current.book.title,
      authors: current.book.authors,
      cover_id: current.book.cover_id,
      year: current.book.year,
      status,
      ...note,
    });
    current.on_shelf = status;
    current.shelf_id = created.id;
    const hit = items.value.find((row) => row.ol_work_key === current.book.ol_work_key);
    if (hit) {
      hit.on_shelf = status;
      hit.shelf_id = created.id;
    }
    toast.show(`Added to ${STATUS_LABEL[status]}`);
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      current.on_shelf = err.item.status;
      current.shelf_id = err.item.id;
      clubMoving.value = true;
      return;
    }
    toast.show(err instanceof ApiError ? err.message : "Could not add that book");
  }
}

async function moveClubPick(status: Status, note: FinishNote = {}) {
  const current = clubPick.value;
  clubMoving.value = false;
  if (!current?.shelf_id) return;
  try {
    await api.patchShelf(current.shelf_id, { status, position: 0, ...note });
    current.on_shelf = status;
    const hit = items.value.find((row) => row.ol_work_key === current.book.ol_work_key);
    if (hit) hit.on_shelf = status;
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not move that book");
  }
}

function startFinish(
  mode: "add" | "move" | "club-add" | "club-move",
  status: "finished" | "did_not_finish",
  title: string,
  hit: SearchHit | null = null,
  item: ShelfItem | null = null,
) {
  finishing.value = { mode, status, title, hit, item };
  adding.value = null;
  moving.value = null;
  existing.value = null;
  clubAdding.value = false;
  clubMoving.value = false;
}

async function saveLibraryFinish(note: FinishNote) {
  const pending = finishing.value;
  finishing.value = null;
  if (!pending) return;
  adding.value = pending.hit ?? null;
  existing.value = pending.item ?? null;
  moving.value = pending.hit ?? null;
  if (pending.mode === "add") {
    await add(pending.status, note);
    return;
  }
  if (pending.mode === "move") {
    await moveExisting(pending.status, note);
    return;
  }
  if (pending.mode === "club-add") {
    await addClubPick(pending.status, note);
    return;
  }
  await moveClubPick(pending.status, note);
}

function cancelLibraryFinish() {
  finishing.value = null;
}

async function add(status: Status, note: FinishNote = {}) {
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
      ...note,
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

async function moveExisting(status: Status, note: FinishNote = {}) {
  const item = existing.value;
  const hit = moving.value;
  const id = item?.id ?? hit?.shelf_id;
  moving.value = null;
  existing.value = null;
  if (!id) return;
  try {
    await api.patchShelf(id, { status, position: 0, ...note });
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
  void loadClubPick();
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
    <ClubPickBanner
      :pick="clubPick"
      compact
      empty-hint="Choose a book below, then set it as the club pick."
      @add="clubAdding = true"
      @move="clubMoving = true"
      @open="router.push('/')"
    />
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
      :club-pick-label="adding.club_pick ? 'Already the club pick' : 'Set as club pick'"
      nominate-label="Nominate for next up"
      @pick="add"
      @finish="(status) => adding && startFinish('add', status, adding.title, adding)"
      @club-pick="startClubPick(adding)"
      @nominate="nominate(adding)"
      @close="adding = null"
    />
    <StatusSheet
      v-if="moving"
      :title="existing ? 'Already on your shelf — move it?' : 'Move to…'"
      :current="existing?.status ?? moving.on_shelf"
      :club-pick-label="moving.club_pick ? 'Already the club pick' : 'Set as club pick'"
      nominate-label="Nominate for next up"
      @pick="moveExisting"
      @finish="(status) => moving && startFinish('move', status, moving.title, moving, existing)"
      @club-pick="startClubPick(moving)"
      @nominate="nominate(moving)"
      @close="moving = null; existing = null"
    />
    <StatusSheet
      v-if="clubAdding && clubPick"
      title="Add the club pick"
      current="currently_reading"
      @pick="addClubPick"
      @finish="(status) => clubPick && startFinish('club-add', status, clubPick.book.title)"
      @close="clubAdding = false"
    />
    <ClubPickSheet
      v-if="settingPick"
      title="Set club pick"
      :book-title="settingPick.title"
      :timezone="clubTimezone"
      :meeting-local="clubPick?.book.ol_work_key === settingPick.ol_work_key ? clubPick.meeting_local : ''"
      @confirm="confirmClubPick"
      @close="settingPick = null"
    />
    <StatusSheet
      v-if="clubMoving && clubPick"
      title="Move the club pick"
      :current="clubPick.on_shelf"
      @pick="moveClubPick"
      @finish="(status) => clubPick && startFinish('club-move', status, clubPick.book.title)"
      @close="clubMoving = false"
    />
    <FinishNoteSheet
      v-if="finishing"
      :title="finishing.status === 'finished' ? 'Finished' : 'Did not finish'"
      :book-title="finishing.title"
      :status="finishing.status"
      @confirm="saveLibraryFinish"
      @close="cancelLibraryFinish"
    />
  </section>
</template>
