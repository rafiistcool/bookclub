<script setup lang="ts">
import { HeartIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BottomSheet from "../components/BottomSheet.vue";
import FavoritePortrait from "../components/FavoritePortrait.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { tp } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { clubDate, orderedShelf, readingMonth, shelfDropPosition, visibleShelf } from "../shelf";
import type { ClubPick, Favorite, ShelfItem } from "../types";

type Filter = "all" | Status;

const { t } = useI18n();
const session = useSession();
const toast = useToast();

const items = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
const showFavorites = ref(false);
const error = ref("");
const loaded = ref(false);
const filter = ref<Filter>("all");
const removing = ref<ShelfItem | null>(null);
const settingPick = ref<ShelfItem | null>(null);
const clubPick = ref<ClubPick | null>(null);
const clubTimezone = ref("UTC");
const saving = ref(false);
const editing = ref<{ item: ShelfItem; action: "status" | "date" } | null>(null);
const dateValue = ref("");
const datePrecision = ref<"month" | "day">("month");
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

async function onReordered(item: ShelfItem, position: number, month?: string) {
  if (saving.value) return;
  const changedMonth = month !== undefined && month !== readingMonth(item, clubTimezone.value);
  if (changedMonth && item.status !== "finished") return;
  const finishedOn = month ? `${month}-01` : null;
  const snapshot = items.value;
  const rows = orderedShelf(items.value).filter((row) => row.id !== item.id);
  rows.splice(Math.min(position, rows.length), 0, changedMonth
    ? { ...item, finished_at: finishedOn ? `${finishedOn}T12:00:00Z` : null } : item);
  items.value = rows.map((row, index) => ({ ...row, shelf_position: index }));
  saving.value = true;
  try {
    const saved = await api.patchShelf(item.id, { shelf_position: position, ...(changedMonth ? { finished_on: finishedOn } : {}) });
    items.value = items.value.map((row) => row.id === saved.id ? saved : row);
    toast.show(t(changedMonth ? "shelf.dateSaved" : "shelf.orderSaved"));
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
    datePrecision.value = "month";
    dateValue.value = clubDate(item.finished_at, clubTimezone.value).slice(0, 7);
    return;
  }
  const rows = visibleShelf(items.value, filter.value, clubTimezone.value).filter(row =>
    item.status === "finished"
      ? row.status === "finished" && readingMonth(row, clubTimezone.value) === readingMonth(item, clubTimezone.value)
      : row.status !== "finished");
  const index = rows.findIndex((row) => row.id === item.id);
  const target = index + (action === "previous" ? -1 : 1);
  if (target < 0 || target >= rows.length) return;
  rows.splice(index, 1);
  rows.splice(target, 0, item);
  editing.value = null;
  void onReordered(item, shelfDropPosition(items.value, rows, item));
}

async function changeStatus(status: Status) {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  editing.value = null;
  saving.value = true;
  try {
    const saved = await api.patchShelf(item.id, { status, position: 0 });
    // The API maintains the old per-status positions for the initial order.
    const oldStatus = item.status;
    items.value = items.value.map((row) => {
      if (row.id === saved.id) return saved;
      if (row.status === status) return { ...row, position: row.position + 1 };
      if (row.status === oldStatus && row.position > item.position) return { ...row, position: row.position - 1 };
      return row;
    });
    toast.show(t("shelf.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.moveFailed"));
  } finally {
    saving.value = false;
  }
}

function bookAction(action: "remove" | "pick" | "nominate") {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  editing.value = null;
  if (action === "remove") removing.value = item;
  else if (action === "pick") settingPick.value = item;
  else void nominate(item);
}

function changeDatePrecision() {
  if (!dateValue.value) return;
  dateValue.value = datePrecision.value === "month" ? dateValue.value.slice(0, 7) : `${dateValue.value.slice(0, 7)}-01`;
}

async function saveDate(clear = false) {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  const finishedOn = datePrecision.value === "month" ? `${dateValue.value}-01` : dateValue.value;
  if (!clear && (!dateValue.value || finishedOn > today.value)) return;
  saving.value = true;
  try {
    const saved = await api.patchShelf(item.id, { finished_on: clear ? null : finishedOn });
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
  <section class="shelf-page">
    <div class="page-head shelf-head">
      <Avatar :username="session.user?.username ?? ''" :src="session.user?.avatar_url" size="lg" />
      <div>
        <h1>{{ t("shelf.title") }}</h1>
        <p class="lede">
          {{ t("shelf.lede", { books: tp("shelf.books", counts.all), reading: counts.currently_reading }) }}
        </p>
      </div>
      <button v-if="loaded && !error" type="button" class="icon-btn favorites-toggle"
        :aria-label="t('favorites.portraitLabel')" :aria-expanded="showFavorites" aria-controls="shelf-favorites"
        @click="showFavorites = !showFavorites">
        <HeartIcon aria-hidden="true" />
      </button>
    </div>
    <div v-if="loaded && !error && showFavorites" id="shelf-favorites" class="shelf-favorites">
      <FavoritePortrait :items="favorites" :empty-hint="t('favorites.emptyOwn')" />
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
      <div class="segmented shelf-filter" role="group" :aria-label="t('shelf.filterByStatus')">
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

      <p class="visually-hidden shelf-drag-hint">{{ t(filter === 'finished' || filter === 'all' ? 'shelf.chronologicalHint' : 'shelf.directDragHint') }}</p>
      <ShelfBoard
        :items="items" :filter="filter" :disabled="saving" :timezone="clubTimezone"
        :club-pick-key="clubPick?.book.ol_work_key"
        @reordered="onReordered" @action="onAction"
      />
    </template>

    <BottomSheet v-if="editing" :title="t(editing.action === 'date' ? 'shelf.editDate' : 'shelf.changeStatus')"
      @close="!saving && (editing = null)">
      <p>{{ editing.item.book.title }}</p>
      <div v-if="editing.action === 'status'" class="stack">
        <button v-for="status in STATUSES" :key="status" type="button" class="btn btn-ghost btn-block"
          :disabled="saving || status === editing.item.status" @click="changeStatus(status)">{{ statusLabel(status) }}</button>
        <details class="shelf-more-actions">
          <summary>{{ t('shelf.moreActions') }}</summary>
          <div class="stack">
            <button v-if="editing.item.status === 'finished'" type="button" class="btn btn-ghost btn-block"
              @click="onAction(editing.item, 'date')">{{ t('shelf.editDate') }}</button>
            <button type="button" class="btn btn-ghost btn-block"
              @click="onAction(editing.item, 'previous')">{{ t('shelf.movePrevious') }}</button>
            <button type="button" class="btn btn-ghost btn-block"
              @click="onAction(editing.item, 'next')">{{ t('shelf.moveNext') }}</button>
            <button type="button" class="btn btn-ghost btn-block" @click="bookAction('pick')">{{ t('card.setPick') }}</button>
            <button type="button" class="btn btn-ghost btn-block" @click="bookAction('nominate')">{{ t('card.nominate') }}</button>
            <button type="button" class="btn btn-ghost btn-block" @click="bookAction('remove')">{{ t('common.remove') }}</button>
          </div>
        </details>
        <button type="button" class="btn btn-ghost btn-block" @click="editing = null">{{ t('common.cancel') }}</button>
      </div>
      <form v-else class="stack" @submit.prevent="saveDate()">
        <label for="date-precision">{{ t('shelf.datePrecision') }}</label>
        <select id="date-precision" v-model="datePrecision" :disabled="saving" @change="changeDatePrecision">
          <option value="month">{{ t('shelf.monthAndYear') }}</option>
          <option value="day">{{ t('shelf.exactDate') }}</option>
        </select>
        <label for="finished-on">{{ t(datePrecision === 'month' ? 'shelf.completionMonth' : 'shelf.finishedOn') }}</label>
        <input id="finished-on" v-model="dateValue" :type="datePrecision === 'month' ? 'month' : 'date'"
          :min="datePrecision === 'month' ? '0001-01' : '0001-01-01'"
          :max="datePrecision === 'month' ? today.slice(0, 7) : today" required :disabled="saving" />
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
.shelf-head { position: relative; padding-right: 42px; margin-bottom: var(--space-6); }
.shelf-head > div { min-width: 0; }
.shelf-head h1 { font-size: clamp(1.5rem, 6.7vw, 2rem); }
.shelf-head .lede { font-size: var(--text-sm); line-height: var(--leading-snug); margin-top: var(--space-1); }
.shelf-head :deep(.avatar) { width: 64px; height: 64px; }
.favorites-toggle { position: absolute; top: 0; right: -6px; color: var(--border-strong); }
.favorites-toggle svg { width: 26px; height: 26px; }
.favorites-toggle[aria-expanded="true"] { color: var(--accent); }
.shelf-favorites { margin-bottom: var(--space-5); }
.shelf-filter { font-family: var(--serif); }
@media (max-width: 359px) {
  .shelf-head :deep(.avatar) { width: 48px; height: 48px; }
  .shelf-head { gap: var(--space-3); }
  .shelf-filter button { padding-inline: var(--space-1); font-size: var(--text-xs); }
}
.shelf-more-actions summary { cursor: pointer; padding-block: var(--space-3); font-size: var(--text-sm); }
.shelf-more-actions .stack { padding-bottom: var(--space-3); }
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
