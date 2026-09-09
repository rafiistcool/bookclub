<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import BookTile from "../components/BookTile.vue";
import BottomSheet from "../components/BottomSheet.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { DESKTOP, useMediaQuery } from "../composables/useMediaQuery";
import { STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { tp } from "../i18n";
import { useToast } from "../stores/toast";
import type { ClubPick, ShelfItem } from "../types";

type Filter = "all" | Status;

const { t } = useI18n();
const desktop = useMediaQuery(DESKTOP);
const toast = useToast();

const items = ref<ShelfItem[]>([]);
const error = ref("");
const loaded = ref(false);
const filter = ref<Filter>("all");
const removing = ref<ShelfItem | null>(null);
const settingPick = ref<ShelfItem | null>(null);
const clubPick = ref<ClubPick | null>(null);
const clubTimezone = ref("UTC");

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

const visible = computed(() => {
  const rows =
    filter.value === "all"
      ? items.value
      : items.value.filter((item) => item.status === filter.value);
  return [...rows].sort(
    (a, b) =>
      STATUSES.indexOf(a.status) - STATUSES.indexOf(b.status) ||
      a.position - b.position ||
      a.id - b.id,
  );
});

async function load() {
  try {
    items.value = (await api.myShelf()).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("shelf.loadFailed");
  } finally {
    loaded.value = true;
  }
  try {
    const current = await api.clubPick();
    clubPick.value = current.pick;
    clubTimezone.value = current.timezone;
  } catch {
    clubPick.value = null;
  }
}

async function onDropped(item: ShelfItem, status: Status, position: number) {
  const snapshot = items.value.map((row) => ({ ...row }));
  try {
    await api.patchShelf(item.id, { status, position });
    items.value = (await api.myShelf()).items;
    toast.show(t("shelf.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    items.value = snapshot;
    toast.show(err instanceof ApiError ? err.message : t("shelf.moveFailed"));
  }
}

async function confirmRemove() {
  const item = removing.value;
  removing.value = null;
  if (!item) return;
  try {
    await api.removeFromShelf(item.id);
    items.value = items.value.filter((row) => row.id !== item.id);
    toast.show(t("shelf.removed", { title: item.book.title }), {
      label: t("common.undo"),
      run: async () => {
        await api.addToShelf({
          ol_work_key: item.book.ol_work_key,
          title: item.book.title,
          authors: item.book.authors,
          cover_id: item.book.cover_id,
          cover_url: item.book.cover_url,
          year: item.book.year,
          status: item.status,
          rating: item.rating,
          take: item.take,
          dnf_reason: item.dnf_reason,
          progress: item.progress,
        });
        items.value = (await api.myShelf()).items;
      },
    });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.removeFailed"));
  }
}

async function nominate(item: ShelfItem) {
  try {
    await api.nominate({
      ol_work_key: item.book.ol_work_key,
      title: item.book.title,
      authors: item.book.authors,
      cover_id: item.book.cover_id,
      cover_url: item.book.cover_url,
      year: item.book.year,
    });
    toast.show(t("shelf.nominated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.nominateFailed"));
  }
}

async function confirmClubPick(meetingAt: string | null) {
  const item = settingPick.value;
  settingPick.value = null;
  if (!item) return;
  try {
    clubPick.value = await api.setClubPick({
      ol_work_key: item.book.ol_work_key,
      title: item.book.title,
      authors: item.book.authors,
      cover_id: item.book.cover_id,
      cover_url: item.book.cover_url,
      year: item.book.year,
      meeting_at: meetingAt,
    });
    toast.show(t("shelf.setPick"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.setPickFailed"));
  }
}

onMounted(load);
</script>

<template>
  <section>
    <div class="page-head">
      <h1>{{ t("shelf.title") }}</h1>
      <p class="lede">
        {{ t("shelf.lede", { books: tp("shelf.books", counts.all), reading: counts.currently_reading }) }}
      </p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div v-if="!loaded" class="book-grid" aria-hidden="true">
      <TileSkeleton :count="9" />
    </div>

    <div v-else-if="items.length === 0" class="empty">
      <h3>{{ t("shelf.emptyTitle") }}</h3>
      <p>{{ t("shelf.emptyBody") }}</p>
      <div class="btn-row">
        <RouterLink class="btn btn-primary" to="/discover">{{ t("home.findBook") }}</RouterLink>
        <RouterLink class="btn btn-ghost" to="/settings">{{ t("shelf.importGoodreads") }}</RouterLink>
      </div>
    </div>

    <template v-else>
      <div v-if="!desktop" class="segmented shelf-filter" role="group" :aria-label="t('shelf.filterByStatus')">
        <button
          type="button"
          :aria-pressed="filter === 'all'"
          @click="filter = 'all'"
        >
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

      <ShelfBoard
        v-if="desktop"
        :items="items"
        :club-pick-key="clubPick?.book.ol_work_key"
        @dropped="onDropped"
        @remove="removing = $event"
        @club-pick="settingPick = $event"
        @nominate="nominate"
      />

      <template v-else>
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
            :club-pick="item.book.ol_work_key === clubPick?.book.ol_work_key"
            show-authors
          />
        </div>
        <p v-else class="fine subtle">
          {{ t("shelf.nothingIn", { status: statusLabel(filter as Status) }) }}
        </p>
      </template>
    </template>

    <MeetingSheet
      v-if="settingPick"
      :title="t('shelf.setAsPick')"
      :book-title="settingPick.book.title"
      :timezone="clubTimezone"
      :blurb="t('shelf.pickBlurb')"
      @confirm="confirmClubPick"
      @close="settingPick = null"
    />

    <BottomSheet
      v-if="removing"
      :title="t('shelf.removeTitle', { title: removing.book.title })"
      @close="removing = null"
    >
      <p class="muted remove-blurb">
        {{ t("shelf.removeBlurb") }}
      </p>
      <div class="stack">
        <button class="btn btn-danger btn-block" type="button" @click="confirmRemove">
          {{ t("common.remove") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" @click="removing = null">
          {{ t("common.cancel") }}
        </button>
      </div>
    </BottomSheet>
  </section>
</template>

<style scoped>
.shelf-filter {
  margin-bottom: var(--space-5);
}

.remove-blurb {
  margin-bottom: var(--space-4);
}
</style>
