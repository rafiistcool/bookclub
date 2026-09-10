<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookTile from "../components/BookTile.vue";
import FavoritePortrait from "../components/FavoritePortrait.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { tp } from "../i18n";
import type { Favorite, ShelfItem } from "../types";

type Filter = "all" | Status;

const { t } = useI18n();
const route = useRoute();
const avatarUrl = ref<string | null>(null);
const username = ref("");
const items = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
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
    avatarUrl.value = shelf.user.avatar_url;
    items.value = shelf.items;
    favorites.value = shelf.favorites;
    error.value = "";
  } catch (err) {
    items.value = [];
    favorites.value = [];
    error.value = err instanceof ApiError ? err.message : t("memberShelf.loadFailed");
  } finally {
    loaded.value = true;
  }
}

onMounted(load);
watch(() => route.params.username, load);
</script>

<template>
  <section>
    <p class="fine back-link"><RouterLink to="/club">{{ t("memberShelf.back") }}</RouterLink></p>
    <div class="page-head member-head">
      <Avatar :username="username" :src="avatarUrl" size="lg" />
      <div>
        <h1>{{ t("memberShelf.title", { name: username }) }}</h1>
        <p v-if="loaded && !error" class="lede nums">
          {{ t("memberShelf.lede", { books: tp("shelf.books", counts.all), reading: counts.currently_reading }) }}
        </p>
      </div>
    </div>
    <FavoritePortrait v-if="loaded && !error" :items="favorites" />

    <p v-if="error" class="error">{{ error }}</p>

    <div v-else-if="!loaded" class="book-grid" aria-hidden="true">
      <TileSkeleton :count="9" />
    </div>

    <div v-else-if="items.length === 0" class="empty">
      <h3>{{ t("memberShelf.emptyTitle") }}</h3>
      <p>{{ t("memberShelf.emptyBody", { name: username }) }}</p>
    </div>

    <template v-else>
      <div class="segmented shelf-filter" role="group" :aria-label="t('shelf.filterByStatus')">
        <button type="button" :aria-pressed="filter === 'all'" @click="filter = 'all'">
          {{ t("common.all") }} <span class="seg-count nums">{{ counts.all }}</span>
        </button>
        <button
          v-for="status in STATUSES"
          :key="status"
          type="button"
          :aria-pressed="filter === status"
          @click="filter = status"
        >
          {{ statusShort(status) }}
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
          :image-url="item.book.cover_url"
          :status="item.status"
          :rating="item.rating"
          show-authors
        />
      </div>
      <p v-else class="fine subtle">
        {{ t("memberShelf.nothingIn", { status: statusLabel(filter as Status) }) }}
      </p>
    </template>
  </section>
</template>

<style scoped>
.back-link {
  margin-bottom: var(--space-4);
}

.member-head {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.shelf-filter {
  margin-bottom: var(--space-5);
}
</style>
