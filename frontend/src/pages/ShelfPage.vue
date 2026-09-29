<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BottomSheet from "../components/BottomSheet.vue";
import FavoritePortrait from "../components/FavoritePortrait.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { DESKTOP, useMediaQuery } from "../composables/useMediaQuery";
import { STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { tp } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { clubDate, dropPosition, shelfGroups } from "../shelf";
import type { ClubPick, Favorite, ShelfItem } from "../types";

type Filter = "all" | Status;

const { t } = useI18n();
const desktop = useMediaQuery(DESKTOP);
const session = useSession();
const toast = useToast();

const items = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
const error = ref("");
const loaded = ref(false);
const filter = ref<Filter>("all");
const removing = ref<ShelfItem | null>(null);
const settingPick = ref<ShelfItem | null>(null);
const clubPick = ref<ClubPick | null>(null);
const clubTimezone = ref("UTC");
const organizing = ref(false);
const saving = ref(false);
const editing = ref<{ item: ShelfItem; action: "status" | "date" } | null>(null);
const dateValue = ref("");
const today = computed(() => clubDate(new Date().toISOString(), clubTimezone.value));

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

async function load() {
  try {
    const shelf = await api.myShelf();
    items.value = shelf.items;
    favorites.value = shelf.favorites;
    clubTimezone.value = shelf.timezone || "UTC";
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("shelf.loadFailed");
  } finally {
    loaded.value = true;
  }
  try {
    const current = await api.clubPick();
    clubPick.value = current.pick;
  } catch {
    clubPick.value = null;
  }
}

async function onDropped(item: ShelfItem, status: Status, position: number) {
  if (saving.value) return;
  const snapshot = items.value.map((row) => ({ ...row }));
  saving.value = true;
  // Mirror the server's per-status order immediately, including its source gap.
  const siblings = items.value.filter((row) => row.status === status && row.id !== item.id)
    .sort((a, b) => a.position - b.position || a.id - b.id);
  const moved = { ...item, status };
  if (status !== item.status) moved.finished_at = status === "finished" ? new Date().toISOString() : null;
  siblings.splice(Math.min(position, siblings.length), 0, moved);
  siblings.forEach((row, index) => { row.position = index; });
  const remaining = items.value.filter((row) => row.status !== status && row.id !== item.id);
  if (item.status !== status) remaining.filter((row) => row.status === item.status)
    .sort((a, b) => a.position - b.position || a.id - b.id)
    .forEach((row, index) => { row.position = index; });
  items.value = [...remaining, ...siblings];
  try {
    const saved = await api.patchShelf(item.id, { status, position });
    items.value = items.value.map((row) => row.id === saved.id ? saved : row);
    toast.show(t(status === item.status ? "shelf.orderSaved" : "shelf.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    items.value = snapshot;
    toast.show(err instanceof ApiError ? err.message : t("shelf.moveFailed"));
  } finally {
    saving.value = false;
  }
}

function onAction(item: ShelfItem, action: "status" | "date" | "previous" | "next") {
  if (saving.value) return;
  if (action === "status" || action === "date") {
    editing.value = { item, action };
    dateValue.value = clubDate(item.finished_at, clubTimezone.value);
    return;
  }
  const group = shelfGroups(items.value, clubTimezone.value).find((group) => group.items.some((row) => row.id === item.id));
  if (!group) return;
  const rows = [...group.items];
  const index = rows.findIndex((row) => row.id === item.id);
  const target = index + (action === "previous" ? -1 : 1);
  if (target < 0 || target >= rows.length) return;
  rows.splice(index, 1);
  rows.splice(target, 0, item);
  void onDropped(item, item.status, dropPosition(items.value, rows, item, item.status));
}

function changeStatus(status: Status) {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  editing.value = null;
  void onDropped(item, status, 0);
}

async function saveDate(clear = false) {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  if (!clear && (!dateValue.value || dateValue.value > today.value)) return;
  saving.value = true;
  try {
    const saved = await api.patchShelf(item.id, { finished_on: clear ? null : dateValue.value });
    items.value = items.value.map((row) => row.id === saved.id ? saved : row);
    editing.value = null;
    toast.show(t("shelf.dateSaved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.moveFailed"));
  } finally {
    saving.value = false;
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
        const shelf = await api.myShelf();
        items.value = shelf.items;
        favorites.value = shelf.favorites;
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
    <div class="page-head shelf-head">
      <Avatar :username="session.user?.username ?? ''" :src="session.user?.avatar_url" size="lg" />
      <div>
        <h1>{{ t("shelf.title") }}</h1>
        <p class="lede">
          {{ t("shelf.lede", { books: tp("shelf.books", counts.all), reading: counts.currently_reading }) }}
        </p>
      </div>
    </div>
    <FavoritePortrait
      v-if="loaded && !error"
      :items="favorites"
      :empty-hint="t('favorites.emptyOwn')"
    />

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

      <button v-if="!desktop" class="btn btn-ghost organize-button" type="button"
        :aria-pressed="organizing" :disabled="saving" @click="organizing = !organizing">
        {{ organizing ? t('shelf.doneOrdering') : t('shelf.organize') }}
      </button>
      <p v-if="!desktop && organizing" class="fine subtle">{{ t('shelf.organizeHint') }}</p>
      <ShelfBoard
        :items="items" :grid="!desktop" :organizing="!desktop && organizing"
        :filter="desktop ? 'all' : filter" :timezone="clubTimezone" :disabled="saving"
        :club-pick-key="clubPick?.book.ol_work_key"
        @dropped="onDropped" @action="onAction"
        @remove="removing = $event" @club-pick="settingPick = $event" @nominate="nominate"
      />
    </template>

    <BottomSheet v-if="editing" :title="t(editing.action === 'date' ? 'shelf.editDate' : 'shelf.changeStatus')"
      @close="!saving && (editing = null)">
      <p>{{ editing.item.book.title }}</p>
      <div v-if="editing.action === 'status'" class="stack">
        <button v-for="status in STATUSES" :key="status" type="button" class="btn btn-ghost btn-block"
          :disabled="saving || status === editing.item.status" @click="changeStatus(status)">{{ statusLabel(status) }}</button>
      </div>
      <form v-else class="stack" @submit.prevent="saveDate()">
        <label for="finished-on">{{ t('shelf.finishedOn') }}</label>
        <input id="finished-on" v-model="dateValue" type="date" :max="today" required :disabled="saving" />
        <button class="btn btn-primary" type="submit" :disabled="saving || !dateValue || dateValue > today">{{ t('common.save') }}</button>
        <button class="btn btn-ghost" type="button" :disabled="saving" @click="saveDate(true)">{{ t('shelf.clearDate') }}</button>
      </form>
    </BottomSheet>

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
.organize-button { margin-bottom: var(--space-4); }
.shelf-head {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.shelf-filter {
  margin-bottom: var(--space-5);
}

.remove-blurb {
  margin-bottom: var(--space-4);
}
</style>
