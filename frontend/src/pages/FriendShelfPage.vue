<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import ActivityFeed from "../components/ActivityFeed.vue";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import SegmentedControl from "../components/SegmentedControl.vue";
import ShelfList from "../components/ShelfList.vue";
import Skeleton from "../components/Skeleton.vue";
import StarRating from "../components/StarRating.vue";
import { formatDate } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useSession } from "../stores/session";
import type { ShelfItem } from "../types";

const route = useRoute();
const flow = useFlow();
const pickStore = usePick();
const session = useSession();

const username = ref("");
const items = ref<ShelfItem[] | null>(null);
const error = ref("");
const tab = ref<"shelf" | "finished" | "activity">("shelf");

const isMe = computed(() => session.user?.username === username.value);
const finished = computed(() =>
  (items.value ?? [])
    .filter((row) => row.status === "finished")
    .sort((a, b) => (b.finished_at ?? b.updated_at).localeCompare(a.finished_at ?? a.updated_at)),
);
const reading = computed(() => (items.value ?? []).filter((row) => row.status === "currently_reading"));
const avgRating = computed(() => {
  const ratings = finished.value.map((row) => row.rating).filter((r): r is number => Boolean(r));
  return ratings.length ? (ratings.reduce((a, b) => a + b, 0) / ratings.length).toFixed(1) : null;
});

async function load() {
  items.value = null;
  username.value = String(route.params.username || "");
  try {
    const shelf = await api.friendShelf(username.value);
    username.value = shelf.user.username;
    items.value = shelf.items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load that shelf";
    items.value = [];
  }
}

onMounted(() => {
  void load();
  void pickStore.load();
});
watch(() => route.params.username, load);
</script>

<template>
  <section :aria-label="`${username}’s shelf`">
    <header class="pick-hero" style="grid-template-columns: auto 1fr; align-items: center; margin-bottom: 16px">
      <Avatar :username="username || '?'" size="lg" />
      <div class="meta">
        <h1 style="font-size: var(--text-xl)">{{ username }}<span v-if="isMe" class="muted fine"> (you)</span></h1>
        <p v-if="items" class="fine muted">
          {{ items.length }} books · {{ finished.length }} finished<template v-if="avgRating"> · avg ★ {{ avgRating }}</template>
        </p>
      </div>
    </header>

    <div v-if="reading.length" class="chip-row" aria-label="Currently reading" style="margin-bottom: 14px">
      <button
        v-for="row in reading"
        :key="row.id"
        class="chip"
        type="button"
        style="min-height: 44px; padding-left: 6px"
        @click="flow.open({ kind: 'details', book: toRef(row.book) })"
      >
        <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="xs" />
        <span class="clamp-1" style="max-width: 160px">{{ row.book.title }}</span>
        <span v-if="row.progress != null" class="faint">{{ row.progress }}%</span>
      </button>
    </div>

    <SegmentedControl
      :model-value="tab"
      :options="[
        { value: 'shelf', label: 'Shelf' },
        { value: 'finished', label: 'Finished', count: finished.length },
        { value: 'activity', label: 'Activity' },
      ]"
      label="Member sections"
      style="margin-bottom: 14px"
      @update:model-value="tab = $event as typeof tab"
    />

    <p v-if="error" class="error">{{ error }}</p>
    <Skeleton v-else-if="!items" kind="cards" :count="3" />
    <template v-else-if="tab === 'shelf'">
      <div v-if="items.length === 0" class="empty"><p>Nothing on this shelf yet.</p></div>
      <ShelfList v-else :items="items" :club-pick-key="pickStore.pick?.book.ol_work_key" :readonly="!isMe" initial="currently_reading" />
    </template>
    <template v-else-if="tab === 'finished'">
      <p v-if="finished.length === 0" class="muted fine">No finished books yet.</p>
      <ul v-else class="list" style="list-style: none; padding: 0">
        <li v-for="row in finished" :key="row.id" class="row-item">
          <button type="button" style="all: unset; cursor: pointer" @click="flow.open({ kind: 'details', book: toRef(row.book) })">
            <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="sm" />
          </button>
          <div class="row-body">
            <h3 class="clamp-1">{{ row.book.title }}</h3>
            <p class="clamp-1">{{ row.book.authors }}<template v-if="row.finished_at"> · {{ formatDate(row.finished_at) }}</template></p>
            <p v-if="row.take" class="clamp-2" style="font-style: italic">“{{ row.take }}”</p>
          </div>
          <StarRating :value="row.rating" />
        </li>
      </ul>
    </template>
    <ActivityFeed v-else :username="username" :limit="30" />
  </section>
</template>
