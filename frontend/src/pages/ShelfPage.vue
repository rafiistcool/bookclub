<script setup lang="ts">
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CalendarIcon,
  CheckIcon,
  FlagIcon,
  HandRaisedIcon,
  HeartIcon,
  Squares2X2Icon,
  TrashIcon,
} from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BottomSheet from "../components/BottomSheet.vue";
import ClubBookActions from "../components/ClubBookActions.vue";
import CoverChooserSheet, {
  announceCoverChange,
  sendPersonalCover,
  type CoverRestore,
} from "../components/CoverChooserSheet.vue";
import FavoritePortrait from "../components/FavoritePortrait.vue";
import { isWithdrawn } from "../components/ReportSheet.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import ShelfViewSwitch from "../components/ShelfViewSwitch.vue";
import TileSkeleton from "../components/TileSkeleton.vue";
import { clubsFor } from "../clubActions";
import { STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { personalCoverKey } from "../covers";
import { isFinishTransition, useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { clubDate, orderedShelf, readingMonth, shelfDropPosition, visibleShelf } from "../shelf";
import type { ClubRef, Favorite, ShelfItem } from "../types";

type Filter = "all" | Status;

const { t } = useI18n();
const session = useSession();
const toast = useToast();
const finish = useFinish();

const items = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
const showFavorites = ref(false);
const error = ref("");
const loaded = ref(false);
const filter = ref<Filter>("all");
const removing = ref<ShelfItem | null>(null);
/** The entry whose own cover the reader is choosing ("Cover wählen"). */
const choosingCover = ref<ShelfItem | null>(null);
/** The chooser's title: a withdrawn book's placeholder in the reader's language. */
const choosingTitle = computed(() => {
  const book = choosingCover.value?.book;
  if (!book) return "";
  return isWithdrawn(book) ? t("report.removedTitle") : book.title;
});
/**
 * The entry whose own photo is being scanned and uploaded: one at a time, and
 * no chooser meanwhile (its Undo would point at the cover before the photo).
 */
const uploadingCoverFor = ref<ShelfItem | null>(null);
/** The current pick of each of the viewer's clubs, for the "Club" badge. */
const clubPicks = ref<{ workId: number; club: ClubRef }[]>([]);
const clubPickIds = computed(() => clubPicks.value.map((row) => row.workId));
const clubActions = ref<InstanceType<typeof ClubBookActions> | null>(null);
const canPick = computed(() => clubsFor("pick", session.user?.clubs).length > 0);
const canNominate = computed(() => clubsFor("nominate", session.user?.clubs).length > 0);
/** Reading dates are days in the owner's zone. */
const shelfTimezone = ref("UTC");
const saving = ref(false);
const editing = ref<{ item: ShelfItem; action: "status" | "date" } | null>(null);
const dateValue = ref("");
const datePrecision = ref<"month" | "day">("month");
const today = computed(() => clubDate(new Date().toISOString(), shelfTimezone.value));

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
    shelfTimezone.value = shelf.timezone || "UTC";
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("shelf.loadFailed");
  } finally {
    loaded.value = true;
  }
  await loadClubPicks();
}

/** Without clubs there is no club call at all (solo mode). */
async function loadClubPicks() {
  if (!session.user?.clubs.length) {
    clubPicks.value = [];
    return;
  }
  try {
    const { items: clubs } = await api.clubs();
    clubPicks.value = clubs.flatMap((club) =>
      club.current_pick ? [{ workId: club.current_pick.book.id, club: { slug: club.slug, name: club.name } }] : [],
    );
  } catch {
    clubPicks.value = [];
  }
}

async function onReordered(item: ShelfItem, position: number, month?: string) {
  if (saving.value) return;
  const changedMonth = month !== undefined && month !== readingMonth(item, shelfTimezone.value);
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
    dateValue.value = clubDate(item.finished_at, shelfTimezone.value).slice(0, 7);
    return;
  }
  const rows = visibleShelf(items.value, filter.value, shelfTimezone.value).filter(row =>
    item.status === "finished"
      ? row.status === "finished" && readingMonth(row, shelfTimezone.value) === readingMonth(item, shelfTimezone.value)
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
    // A book just finished gets its moment instead of the toast; the status sheet is gone by now.
    if (isFinishTransition(oldStatus, saved)) void celebrate(saved);
    else toast.show(t("shelf.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.moveFailed"));
  } finally {
    saving.value = false;
  }
}

/** The moment for a finished book; the stars it saves show on the shelf. */
async function celebrate(item: ShelfItem) {
  const clubSlug = clubPicks.value.find((row) => row.workId === item.book.id)?.club.slug ?? null;
  const rated = await finish.celebrate({ item, clubSlug });
  if (rated) items.value = items.value.map((row) => (row.id === rated.id ? rated : row));
}

function bookAction(action: "remove" | "pick" | "nominate" | "cover") {
  const item = editing.value?.item;
  if (!item || saving.value) return;
  editing.value = null;
  if (action === "remove") {
    removing.value = item;
    return;
  }
  if (action === "cover") {
    // A withdrawn book keeps its cover, as on the book page.
    if (isWithdrawn(item.book)) return;
    if (uploadingCoverFor.value) toast.show(t("cover.waitForUpload"));
    else choosingCover.value = item;
    return;
  }
  clubActions.value?.start(action, {
    id: item.book.id,
    title: item.book.title,
    currentIn: clubPicks.value.filter((row) => row.workId === item.book.id).map((row) => row.club),
  });
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

/**
 * Put a changed entry in its place; one no longer on the shelf stays gone. A
 * favourite shows the entry's cover, so it follows a cover change.
 */
function replaceItem(saved: ShelfItem) {
  items.value = items.value.map((row) => (row.id === saved.id ? saved : row));
  const { cover_url, cover_thumb_url, cover_public_id, spine_color } = saved.book;
  favorites.value = favorites.value.map((row) =>
    row.book.id === saved.book.id
      ? { ...row, book: { ...row.book, cover_url, cover_thumb_url, cover_public_id, spine_color } }
      : row,
  );
}

/** The chooser saved a cover: the shelf shows it, with an Undo that puts the previous one back. */
function onCoverChanged(item: ShelfItem, restore: CoverRestore | null) {
  replaceItem(item);
  announceCoverChange(item.id, restore, replaceItem);
}

/** "Own photo": the chooser closed at once; the page scans and uploads the photo. */
async function onPersonalPhoto(file: File, restore: CoverRestore | null) {
  const entry = choosingCover.value;
  choosingCover.value = null;
  if (!entry) return;
  if (uploadingCoverFor.value) {
    toast.show(t("cover.waitForUpload"));
    return;
  }
  uploadingCoverFor.value = entry;
  try {
    const item = await sendPersonalCover(entry.id, file);
    if (item) onCoverChanged(item, restore);
  } finally {
    uploadingCoverFor.value = null;
  }
}

async function confirmRemove() {
  const item = removing.value;
  removing.value = null;
  if (!item) return;
  // The reader's own cover goes with the entry; Undo chooses it again.
  const coverKey = personalCoverKey({ personal_cover: item.personal_cover, cover_public_id: item.book.cover_public_id });
  try {
    await api.removeFromShelf(item.id);
    items.value = items.value.filter((row) => row.id !== item.id);
    toast.show(t("shelf.removed", { title: item.book.title }), {
      label: t("common.undo"),
      run: async () => {
        const restored = await api.addToShelf({
          work_id: item.book.id,
          edition_id: item.edition?.id ?? null,
          status: item.status,
          rating: item.rating,
          rating_visibility: item.rating_visibility,
          take: item.take,
          dnf_reason: item.dnf_reason,
          progress: item.progress,
          current_page: item.current_page,
        });
        if (coverKey) {
          try {
            await api.chooseCover(restored.id, coverKey);
          } catch (err) {
            toast.show(err instanceof ApiError ? err.message : t("coverChoice.undoFailed"));
          }
        }
        const shelf = await api.myShelf();
        items.value = shelf.items;
        favorites.value = shelf.favorites;
      },
    });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.removeFailed"));
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

    <!-- The chooser is gone while the own photo is scanned and uploaded: this says it is under way, in view while the shelf scrolls. -->
    <p v-if="uploadingCoverFor" class="finer cover-upload-status" role="status">
      {{ t("coverChoice.uploading", { title: isWithdrawn(uploadingCoverFor.book) ? t("report.removedTitle") : uploadingCoverFor.book.title }) }}
    </p>

    <div v-if="loaded && !error" class="shelf-view-toolbar">
      <ShelfViewSwitch active="books" />
      <span class="shelf-extra-links">
        <RouterLink class="chip" to="/diary">{{ t("diaryPage.title") }}</RouterLink>
        <RouterLink class="chip" to="/quotes">{{ t("quotes.title") }}</RouterLink>
        <RouterLink class="chip" to="/stats">{{ t("stats.short") }}</RouterLink>
      </span>
    </div>

    <div v-if="!loaded" class="book-grid" aria-hidden="true">
      <TileSkeleton :count="9" />
    </div>

    <div v-else-if="items.length === 0" class="empty">
      <h3>{{ t("shelf.emptyTitle") }}</h3>
      <div class="btn-row">
        <RouterLink class="btn btn-primary" to="/discover">{{ t("home.findBook") }}</RouterLink>
        <RouterLink class="btn btn-ghost" to="/settings/import">{{ t("shelf.importGoodreads") }}</RouterLink>
      </div>
    </div>

    <template v-else>
      <div class="segmented stacked shelf-filter" role="group" :aria-label="t('shelf.filterByStatus')">
        <button
          type="button"
          :aria-pressed="filter === 'all'"
          @click="filter = 'all'"
        >
          <span class="seg-label">{{ t("common.all") }}</span>
          <span class="seg-count nums">{{ counts.all }}</span>
        </button>
        <button
          v-for="status in STATUSES"
          :key="status"
          type="button"
          :aria-pressed="filter === status"
          @click="filter = status"
        >
          <span class="seg-label">{{ statusShort(status) }}</span>
          <span class="seg-count nums">{{ counts[status] }}</span>
        </button>
      </div>

      <p class="visually-hidden shelf-drag-hint">{{ t(filter === 'finished' || filter === 'all' ? 'shelf.chronologicalHint' : 'shelf.directDragHint') }}</p>
      <ShelfBoard
        :items="items" :filter="filter" :disabled="saving" :timezone="shelfTimezone"
        :club-pick-ids="clubPickIds"
        @reordered="onReordered" @action="onAction"
      />
    </template>

    <BottomSheet v-if="editing" :title="t(editing.action === 'date' ? 'shelf.editDate' : 'shelf.changeStatus')"
      @close="!saving && (editing = null)">
      <p class="sheet-book">{{ editing.item.book.title }}</p>
      <div v-if="editing.action === 'status'">
        <!-- The same short words as the button that opened the sheet. The
             current status is marked, not disabled: it stays readable and
             focusable, and tapping it just closes the sheet. -->
        <ul class="sheet-rows status-choices">
          <li v-for="status in STATUSES" :key="status">
            <button type="button" class="sheet-row status-choice"
              :class="{ current: status === editing.item.status }" :aria-pressed="status === editing.item.status"
              :disabled="saving" @click="status === editing.item.status ? (editing = null) : changeStatus(status)">
              <span class="sheet-label">{{ statusShort(status) }}</span>
              <CheckIcon v-if="status === editing.item.status" class="status-check" aria-hidden="true" />
            </button>
          </li>
        </ul>
        <details class="shelf-more-actions">
          <summary><span>{{ t('shelf.moreActions') }}</span><svg class="more-chevron" width="18" height="18"
            viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"
            stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg></summary>
          <ul class="sheet-rows">
            <li v-if="editing.item.status === 'finished'">
              <button type="button" class="sheet-row" @click="onAction(editing.item, 'date')">
                <CalendarIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('shelf.editDate') }}</span>
              </button>
            </li>
            <li>
              <button type="button" class="sheet-row" @click="onAction(editing.item, 'previous')">
                <ArrowUpIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('shelf.movePrevious') }}</span>
              </button>
            </li>
            <li>
              <button type="button" class="sheet-row" @click="onAction(editing.item, 'next')">
                <ArrowDownIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('shelf.moveNext') }}</span>
              </button>
            </li>
            <li v-if="!isWithdrawn(editing.item.book)">
              <button type="button" class="sheet-row choose-cover" aria-haspopup="dialog" @click="bookAction('cover')">
                <Squares2X2Icon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('coverChoice.title') }}</span>
              </button>
            </li>
            <li v-if="canPick">
              <button type="button" class="sheet-row" @click="bookAction('pick')">
                <FlagIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('card.setPick') }}</span>
              </button>
            </li>
            <li v-if="canNominate">
              <button type="button" class="sheet-row" @click="bookAction('nominate')">
                <HandRaisedIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('card.nominate') }}</span>
              </button>
            </li>
            <li>
              <button type="button" class="sheet-row danger remove-row" @click="bookAction('remove')">
                <TrashIcon class="sheet-icon" aria-hidden="true" /><span class="sheet-label">{{ t('common.remove') }}</span>
              </button>
            </li>
          </ul>
        </details>
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

    <CoverChooserSheet
      v-if="choosingCover"
      :entry-id="choosingCover.id"
      :work-id="choosingCover.book.id"
      :title="choosingTitle"
      :personal-cover="Boolean(choosingCover.personal_cover)"
      :cover-public-id="choosingCover.book.cover_public_id"
      @changed="onCoverChanged"
      @picked="onPersonalPhoto"
      @close="choosingCover = null"
    />

    <!-- A new pick changes the "Club" badges. -->
    <ClubBookActions ref="clubActions" @picked="loadClubPicks" />

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
.shelf-view-toolbar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--space-3); margin-bottom: var(--space-5); }
/* Pills, like the Books/Shelf switch beside them. */
.shelf-extra-links { display: flex; flex-wrap: wrap; gap: var(--space-2); }
.shelf-head { position: relative; padding-right: 42px; margin-bottom: var(--space-6); }
.shelf-head > div { min-width: 0; }
.shelf-head :deep(.avatar) { width: 64px; height: 64px; }
.favorites-toggle { position: absolute; top: 0; right: -6px; color: var(--text-subtle); }
.favorites-toggle svg { width: 26px; height: 26px; }
.favorites-toggle[aria-expanded="true"] { color: var(--accent); }
.shelf-favorites { margin-bottom: var(--space-5); }
.cover-upload-status {
  position: sticky;
  top: calc(var(--header-h) + env(safe-area-inset-top) + var(--space-2));
  z-index: 5;
  width: fit-content;
  max-width: 100%;
  margin: 0 auto var(--space-3);
  padding: var(--space-1) var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-translucent-strong);
  color: var(--text-muted);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
@media (min-width: 1024px) {
  .shelf-filter { max-width: 560px; }
}
@media (max-width: 359px) {
  .shelf-head :deep(.avatar) { width: 48px; height: 48px; }
  .shelf-head { gap: var(--space-3); }
  .shelf-filter button { padding-inline: var(--space-1); font-size: var(--text-xs); }
}
.sheet-book { margin-bottom: var(--space-3); color: var(--text-muted); font-family: var(--serif); }
.status-choice.current { color: var(--accent-hover); }
.status-check { flex: none; width: 20px; height: 20px; }
.shelf-more-actions { border-top: 1px solid var(--border); }
.shelf-more-actions summary {
  display: flex; align-items: center; justify-content: space-between; min-height: var(--tap);
  cursor: pointer; list-style: none; font-size: var(--text-sm); font-weight: 600; color: var(--text-muted);
}
.shelf-more-actions summary::-webkit-details-marker { display: none; }
.more-chevron { transition: transform var(--dur-base) var(--ease-out); }
.shelf-more-actions[open] .more-chevron { transform: rotate(180deg); }
.shelf-more-actions summary { padding: 0 var(--space-1); }
.shelf-more-actions[open] .sheet-rows { animation: more-in var(--dur-base) var(--ease-out); }
.shelf-more-actions .sheet-rows { border-top: 1px solid var(--border); }
@keyframes more-in { from { opacity: 0; transform: translateY(-6px); } }
@media (prefers-reduced-motion: reduce) {
  .shelf-more-actions[open] .sheet-rows { animation: none; }
  .more-chevron { transition: none; }
}
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
