<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { STATUSES, STATUS_LABEL, STATUS_SHORT, type Status } from "../constants";
import type { ShelfItem } from "../types";

type Filter = "all" | Status;

const route = useRoute();
const username = ref("");
const items = ref<ShelfItem[]>([]);
const error = ref("");
const loaded = ref(false);
const filter = ref<Filter>("all");

const counts = computed(() => {
  const tally: Record<Filter, number> = {
    all: items.value.length,
    want_to_read: 0,
    currently_reading: 0,
    finished: 0,
    did_not_finish: 0,
  };
  for (const item of items.value) tally[item.status] += 1;
  return tally;
});

const visible = computed(() =>
  filter.value === "all"
    ? items.value
    : items.value.filter((item) => item.status === filter.value),
);

async function load() {
  loaded.value = false;
  filter.value = "all";
  username.value = String(route.params.username || "");
  try {
    const shelf = await api.friendShelf(username.value);
    username.value = shelf.user.username;
    items.value = shelf.items;
    error.value = "";
  } catch (err) {
    items.value = [];
    error.value = err instanceof ApiError ? err.message : "Could not load that shelf";
  } finally {
    loaded.value = true;
  }
}

onMounted(load);
watch(() => route.params.username, load);
</script>

<template>
  <section>
    <p class="fine back-link"><RouterLink to="/club">← Club</RouterLink></p>
    <div class="page-head">
      <h1>{{ username }}'s shelf</h1>
      <p v-if="loaded && !error" class="lede nums">
        {{ counts.all }} {{ counts.all === 1 ? "book" : "books" }} ·
        {{ counts.currently_reading }} in progress
      </p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div v-else-if="!loaded" class="book-grid" aria-hidden="true">
      <TileSkeleton :count="9" />
    </div>

    <div v-else-if="items.length === 0" class="empty">
      <h3>Nothing on this shelf yet</h3>
      <p>{{ username }} has not added any books.</p>
    </div>

    <template v-else>
      <div class="segmented shelf-filter" role="group" aria-label="Filter by status">
        <button type="button" :aria-pressed="filter === 'all'" @click="filter = 'all'">
          All <span class="seg-count nums">{{ counts.all }}</span>
        </button>
        <button
          v-for="status in STATUSES"
          :key="status"
          type="button"
          :aria-pressed="filter === status"
          @click="filter = status"
        >
          {{ STATUS_SHORT[status] }}
          <span class="seg-count nums">{{ counts[status] }}</span>
        </button>
      </div>

      <div v-if="visible.length" class="book-grid">
        <BookTile
          v-for="item in visible"
          :key="item.id"
          :ol-work-key="item.book.ol_work_key"
          :title="item.book.title"
          :authors="item.book.authors"
          :cover-id="item.book.cover_id"
          :status="item.status"
          :rating="item.rating"
          show-authors
        />
      </div>
      <p v-else class="fine subtle">
        Nothing in {{ STATUS_LABEL[filter as Status] }}.
      </p>
    </template>
  </section>
</template>

<style scoped>
.back-link {
  margin-bottom: var(--space-4);
}

.shelf-filter {
  margin-bottom: var(--space-5);
}
</style>
