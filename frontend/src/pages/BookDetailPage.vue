<script setup lang="ts">
import { EllipsisHorizontalIcon, FlagIcon, HandRaisedIcon, HeartIcon } from "@heroicons/vue/24/outline";
import { HeartIcon as HeartSolidIcon } from "@heroicons/vue/24/solid";
import { vAutosize } from "../autosize";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BookDiary from "../components/BookDiary.vue";
import ProgressEditor from "../components/ProgressEditor.vue";
import { positionOf, positionPatch, positionSaved, type ReadingPosition } from "../progress";
import { whenIdle } from "../whenIdle";
import QuoteList from "../components/QuoteList.vue";
import ClubBookActions from "../components/ClubBookActions.vue";
import ReportSheet, { isWithdrawn, type ReportTarget } from "../components/ReportSheet.vue";
import EditionPicker from "../components/EditionPicker.vue";
import CoverPhotoButton, { sendCoverPhoto, type CoverTarget } from "../components/CoverPhotoButton.vue";
import AdminEditionCover from "../components/AdminEditionCover.vue";
import BookActionsSheet, { type BookAction } from "../components/BookActionsSheet.vue";
import CoverPrompt from "../components/CoverPrompt.vue";
import CoverChooserSheet, {
  announceCoverChange,
  sendPersonalCover,
  type CoverRestore,
} from "../components/CoverChooserSheet.vue";
import MergeRequestSheet from "../components/MergeRequestSheet.vue";
import EditRequestSheet from "../components/EditRequestSheet.vue";
import { clubsFor, pickTargets } from "../clubActions";
import { asksForCoverPhoto, personalCoverKey } from "../covers";
import { backTarget, type BackTarget } from "../backTarget";
import { worthRetrying } from "../retry";
import {
  bookPath,
  parseBookParam,
  STATUSES,
  statusLabel,
  statusShort,
  starLabel,
  type Status,
} from "../constants";
import { tp, type Locale } from "../i18n";
import { formatRating } from "../stats";
import { isFinishTransition, useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { BookDetailOut, BookOut, EditionOut } from "../api/generated";
import type { BookDetail, ClubRef, ShelfItem, Visibility } from "../types";

const { t, locale } = useI18n();

const route = useRoute();
const router = useRouter();
const session = useSession();
const toast = useToast();
const finish = useFinish();

/**
 * The "←" link: back to the page the reader came from, named after it, or to
 * Discover when the book was opened directly. Read again on every route
 * change, since another book reuses this page.
 */
const back = ref<BackTarget>({ kind: "fallback", to: "/discover" });
/** The previous page's path: the link's href, for a new tab. */
const backPath = ref("/discover");
const NAV_KEYS = { home: "nav.home", discover: "nav.discover", shelf: "nav.shelf", club: "nav.club", profile: "nav.profile" } as const;
watch(
  () => route.fullPath,
  () => {
    const state = router.options.history.state;
    const previous = state && typeof state.back === "string" ? state.back : null;
    back.value = backTarget(previous);
    backPath.value = back.value.kind === "back" && previous ? previous : "/discover";
  },
  { immediate: true },
);
const backLabel = computed(() => {
  const target = back.value;
  if (target.kind === "fallback") return t("book.backTo", { page: t("nav.discover") });
  if (target.page === "user") return t("book.backTo", { page: target.username ?? "" });
  if (target.page === null) return t("book.back");
  return t("book.backTo", { page: t(NAV_KEYS[target.page]) });
});
function goBack() {
  if (back.value.kind === "back") router.back();
  else void router.push(back.value.to);
}

const book = ref<BookDetail | null>(null);
const error = ref("");
/** The failed load may work a second time (network, 5xx); a 404 or a bad id never does. */
const retryable = ref(false);
const loaded = ref(false);
const loadWaitSec = ref(0);
const busy = ref(false);
const refreshing = ref(false);
const expanded = ref(false);
/** What the reader is reporting: the cover, or a book a reader added (#54). */
const reporting = ref<ReportTarget | null>(null);
/** A book a reader added whose text an admin removed on a report: its stored placeholder title is shown translated. */
const withdrawn = computed(() => isWithdrawn(book.value));
/** The title as this page shows it: the placeholder of a withdrawn book in the reader's language. */
const shownTitle = computed(() => (withdrawn.value ? t("report.removedTitle") : (book.value?.title ?? "")));
const requestingMerge = ref(false);
const requestingEdit = ref(false);
/** An instance admin's direct edit: of the book, or of the viewer's edition (read fresh). */
const adminEditing = ref<"work" | "edition" | null>(null);
const adminEdition = ref<EditionOut | null>(null);
/** Bumped after an edition edit, so the edition picker reads the editions again. */
const editionsVersion = ref(0);
/** The "…" sheet with everything about the book that is not reading it. */
const moreOpen = ref(false);
const moreButton = ref<HTMLButtonElement | null>(null);
/** An instance admin's list of the work's editions, each with a cover upload. */
const adminEditionCover = ref(false);
/** The reader's own cover of the book on their shelf ("Cover wählen"). */
const choosingCover = ref(false);
/** A photo chosen in a sheet is uploading; the sheet itself is gone by then. */
// The work a photo from a sheet is being uploaded for: the page can move to
// another book (Back) while it runs, and that book is not uploading.
const coverUploadFor = ref<number | null>(null);
const coverUploading = computed(() => coverUploadFor.value !== null && coverUploadFor.value === book.value?.id);

const takeDraft = ref("");
const reasonDraft = ref("");
const diary = ref<InstanceType<typeof BookDiary> | null>(null);
const clubActions = ref<InstanceType<typeof ClubBookActions> | null>(null);

const param = computed(() => String(route.params.id || ""));
const longDescription = computed(() => (book.value?.description.length ?? 0) > 420);
/** A title this long would run ten lines beside the cover at the display size. */
const longTitle = computed(() => shownTitle.value.length > 48);
/** Each author on its own, so a name never breaks inside itself. */
const authorNames = computed(() => (book.value?.authors ?? "").split(", ").filter(Boolean));
const canNominate = computed(() => clubsFor("nominate", session.user?.clubs).length > 0);
/**
 * The clubs this book can still become the pick of. Once it is the pick
 * everywhere the reader may set one, the row goes: the pick badge at the
 * top says so instead of a greyed-out row.
 */
const pickOpen = computed(() => pickTargets(session.user?.clubs, book.value?.club_picks));
/** The row names the club when there is one; with several, the sheet asks which. */
const pickRowLabel = computed(() =>
  pickOpen.value.length === 1 ? t("card.setPickIn", { club: pickOpen.value[0]!.name }) : t("book.setAsPick"),
);
const pickNames = computed(() => (book.value?.club_picks ?? []).map((club) => club.name).join(", "));
const hasCover = computed(() => Boolean(book.value?.cover_url || book.value?.cover_thumb_url));
/** The club the finished moment counts in: the first whose current pick this is. */
const pickClubSlug = computed(() => book.value?.club_picks[0]?.slug ?? null);

function syncDrafts(detail: BookDetail) {
  takeDraft.value = detail.take;
  reasonDraft.value = detail.dnf_reason;
}

/**
 * The book asks the reader for a photo of their copy (#74): the prompt beside
 * the cover takes the place of the small link under it.
 */
const coverPrompted = computed(() => (book.value ? asksForCoverPhoto(book.value) : false));

/**
 * A photo became the book's cover: show it without reloading the page. A
 * reader with their own cover keeps seeing theirs, so then nothing changes.
 */
function adoptCover(updated: BookOut) {
  const detail = book.value;
  if (!detail || detail.personal_cover) return;
  detail.cover_url = updated.cover_url;
  detail.cover_thumb_url = updated.cover_thumb_url;
  detail.cover_public_id = updated.cover_public_id;
}

/** The reader chose a cover (or went back to the standard one): the entry's book carries it. */
function adoptEntryCover(detail: BookDetail, item: ShelfItem) {
  if (detail.shelf_id !== item.id) return;
  detail.cover_url = item.book.cover_url;
  detail.cover_thumb_url = item.book.cover_thumb_url;
  detail.cover_public_id = item.book.cover_public_id;
  detail.personal_cover = Boolean(item.personal_cover);
}

/**
 * Copy the viewer's shelf entry onto the page after a write. The page shows
 * the entry's cover (the reader's own, else its edition's, else the work's),
 * so another edition brings its cover along.
 */
function adoptEntry(detail: BookDetail, item: ShelfItem) {
  detail.shelf_id = item.id;
  adoptEntryCover(detail, item);
  detail.on_shelf = item.status;
  detail.edition_id = item.edition?.id ?? null;
  detail.rating = item.rating;
  detail.rating_visibility = item.rating_visibility;
  detail.take = item.take;
  detail.dnf_reason = item.dnf_reason;
  detail.progress = item.progress;
  detail.current_page = item.current_page;
  detail.progress_pages = item.book.pages;
  syncDrafts(detail);
}

let loadClock: ReturnType<typeof setInterval> | null = null;
let loadAbort: AbortController | null = null;

function stopLoadClock() {
  if (loadClock !== null) clearInterval(loadClock);
  loadClock = null;
  loadWaitSec.value = 0;
}

function startLoadClock() {
  const started = Date.now();
  stopLoadClock();
  loadClock = window.setInterval(() => {
    loadWaitSec.value = Math.floor((Date.now() - started) / 1000);
  }, 500);
}

async function load() {
  const target = parseBookParam(param.value);
  // The legacy key or merged id we just resolved now reads /book/<id>: same book, no second fetch.
  if (target.kind === "id" && book.value?.id === target.id && loaded.value && !error.value) return;
  loaded.value = false;
  error.value = "";
  retryable.value = false;
  expanded.value = false;
  reporting.value = null;
  moreOpen.value = false;
  adminEditionCover.value = false;
  choosingCover.value = false;
  loadAbort?.abort();
  if (target.kind === "invalid") {
    book.value = null;
    error.value = t("book.notFound");
    loaded.value = true;
    return;
  }
  const controller = new AbortController();
  loadAbort = controller;
  startLoadClock();
  try {
    const detail = await api.book(target.kind === "id" ? target.id : target.key, {
      signal: controller.signal,
    });
    if (controller.signal.aborted) return;
    book.value = detail;
    syncDrafts(detail);
    // Old links (/book/OL…W, BC…, ISBN…, GB…) and merged works land on the work's
    // own id; the #entry- hash of a diary link comes along.
    if (target.kind === "legacy" || target.id !== detail.id) {
      void router.replace({ path: bookPath(detail), query: route.query, hash: route.hash });
    }
  } catch (err) {
    if (
      (err instanceof DOMException && err.name === "AbortError") ||
      (err instanceof Error && err.name === "AbortError")
    ) {
      return;
    }
    book.value = null;
    error.value = err instanceof ApiError ? err.message : t("book.loadFailed");
    retryable.value = worthRetrying(err);
  } finally {
    if (!controller.signal.aborted) {
      loaded.value = true;
      stopLoadClock();
    }
  }
}

async function setStatus(status: Status) {
  const detail = book.value;
  // The current segment is a state, not an action: tapping it again changed
  // nothing but still moved the book to the top of its column.
  if (!detail || busy.value || detail.on_shelf === status) return;
  const previous = detail.on_shelf;
  busy.value = true;
  try {
    const item = detail.shelf_id
      ? await api.patchShelf(detail.shelf_id, { status, position: 0 })
      : await api.addToShelf({ work_id: detail.id, status });
    adoptEntry(detail, item);
    // The diary's guards follow the reader's status (finished sees everything).
    void diary.value?.reload();
    // Finished just now: the moment instead of the toast. A 409 retry whose
    // entry was finished already returned above, without either.
    if (isFinishTransition(previous, item)) celebrate(item);
    else toast.show(t("book.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    // Another tab or the importer may have shelved it since this page loaded.
    if (err instanceof ApiError && err.status === 409 && err.item) {
      adoptEntry(detail, err.item);
      busy.value = false;
      await setStatus(status);
      return;
    }
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

async function patch(
  body: Parameters<typeof api.patchShelf>[1],
  message: string,
  action?: { label: string; run: () => void | Promise<void> },
) {
  const detail = book.value;
  if (!detail?.shelf_id || busy.value) return;
  busy.value = true;
  try {
    adoptEntry(detail, await api.patchShelf(detail.shelf_id, body));
    toast.show(message, action);
    if ("rating" in body || "rating_visibility" in body) void refreshClubRating();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** Progress from the sheet, with an Undo; the diary's spoiler shields follow it. */
// Only the newest progress save can be undone (an older toast's "before"
// would undo the newer save too).
let undoToast = 0;

async function saveProgress(value: ReadingPosition, undo = true) {
  // An Undo waits for a save still under way instead of being dropped.
  if (!undo) await whenIdle(busy);
  const detail = book.value;
  if (!detail?.shelf_id || busy.value) return;
  // The server keeps progress only while the book is being read.
  if (!undo && detail.on_shelf !== "currently_reading") return;
  const before = positionOf(detail);
  busy.value = true;
  try {
    const item = await api.patchShelf(detail.shelf_id, positionPatch(value));
    adoptEntry(detail, item);
    void diary.value?.reload();
    if (undoToast) toast.dismiss(undoToast);
    undoToast = 0;
    if (undo) {
      undoToast = toast.show(positionSaved(positionOf(item)), {
        label: t("common.undo"),
        run: () => saveProgress(before, false),
      });
    } else toast.show(t("progress.undone"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

function celebrate(item: ShelfItem) {
  void finish.celebrate({ item, clubSlug: pickClubSlug.value });
}

/**
 * The progress sheet's "Mark as read" saved the status and opened the moment
 * itself. The progress it left is gone on the server, so an Undo of an
 * earlier progress save would point at a state that no longer exists.
 */
function onFinished(item: ShelfItem) {
  const detail = book.value;
  if (!detail) return;
  if (undoToast) toast.dismiss(undoToast);
  undoToast = 0;
  adoptEntry(detail, item);
  void diary.value?.reload();
  void refreshClubRating();
}

// The moment saved stars, a line or "only me": the page shows them, and the
// club's average moves with them.
watch(
  () => finish.lastSaved,
  (saved) => {
    const detail = book.value;
    if (!saved || !detail || detail.shelf_id !== saved.id) return;
    adoptEntry(detail, saved);
    void refreshClubRating();
  },
);

/** The average moved with my stars; pull it without disturbing the page. */
async function refreshClubRating() {
  const detail = book.value;
  if (!detail) return;
  try {
    const fresh = await api.book(detail.id);
    detail.club_rating = fresh.club_rating;
    detail.rating_count = fresh.rating_count;
    detail.readers = fresh.readers;
  } catch {
    /* Cosmetic; the next load catches up. */
  }
}

/** A second tap on the chosen star keeps it; "Remove rating" is the way to clear it. */
function rate(value: number) {
  const detail = book.value;
  if (!detail || detail.rating === value) return;
  void patch({ rating: value }, t("book.starsOf", { n: value }), {
    label: t("book.writeAFew"),
    run: () => diary.value?.focusComposer(),
  });
}

function clearRating() {
  void patch({ rating: null }, t("book.ratingCleared"));
}

async function setRatingVisibility(event: Event) {
  const box = event.target as HTMLInputElement;
  const visibility: Visibility = box.checked ? "only_me" : "audience";
  await patch({ rating_visibility: visibility }, t("book.ratingVisibilitySaved"));
  // The box shows what the server stored: after a failed save the click is undone.
  box.checked = book.value?.rating_visibility === "only_me";
}

async function remove() {
  const detail = book.value;
  if (!detail?.shelf_id) return;
  // The reader's own cover goes with the entry; Undo chooses it again.
  const coverKey = personalCoverKey(detail);
  const previous = {
    work_id: detail.id,
    edition_id: detail.edition_id,
    status: detail.on_shelf as Status,
    rating: detail.rating,
    rating_visibility: detail.rating_visibility,
    take: detail.take,
    dnf_reason: detail.dnf_reason,
    progress: detail.progress,
    current_page: detail.current_page,
  };
  busy.value = true;
  try {
    await api.removeFromShelf(detail.shelf_id);
    detail.shelf_id = null;
    detail.on_shelf = null;
    detail.edition_id = null;
    detail.rating = null;
    detail.rating_visibility = null;
    detail.take = "";
    detail.dnf_reason = "";
    detail.progress = null;
    detail.current_page = null;
    syncDrafts(detail);
    // Off the shelf, no position: the diary guards every post written further on again.
    void diary.value?.reload();
    // The entry's cover (its own, or its edition's) goes with it.
    if (coverKey || previous.edition_id != null) void showStandardCover(detail);
    toast.show(t("book.removed"), {
      label: t("common.undo"),
      run: async () => {
        const item = await api.addToShelf(previous);
        adoptEntry(detail, item);
        void diary.value?.reload();
        if (coverKey) await reapplyCover(detail, item.id, coverKey);
      },
    });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.removeFailed"));
  } finally {
    busy.value = false;
  }
}

/** Off the shelf, the page shows the cover everyone sees again. */
async function showStandardCover(detail: BookDetail) {
  detail.personal_cover = false;
  try {
    const fresh = await api.book(detail.id);
    // Added back meanwhile (Undo): the entry's cover is the page's again.
    if (detail.shelf_id !== null) return;
    detail.cover_url = fresh.cover_url;
    detail.cover_thumb_url = fresh.cover_thumb_url;
    detail.cover_public_id = fresh.cover_public_id;
  } catch {
    /* Cosmetic; the next load catches up. */
  }
}

/** Undo of a removal: the entry is back, and so is the reader's own cover. */
async function reapplyCover(detail: BookDetail, entryId: number, key: string) {
  try {
    adoptEntryCover(detail, await api.chooseCover(entryId, key));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("coverChoice.undoFailed"));
  }
}

/** The chooser saved a cover: show it, with an Undo that puts the previous one back. */
function onCoverChanged(item: ShelfItem, restore: CoverRestore | null) {
  const detail = book.value;
  if (!detail) return;
  adoptEntryCover(detail, item);
  announceCoverChange(item.id, restore, (restored) => {
    if (book.value === detail) adoptEntryCover(detail, restored);
  });
}

/** "Own photo" in the chooser: it closed at once; the page scans and uploads the photo. */
async function onPersonalPhoto(file: File, restore: CoverRestore | null) {
  const detail = book.value;
  await closeSheets();
  const entryId = detail?.shelf_id;
  if (!detail || entryId == null) return;
  if (coverUploadFor.value !== null) {
    toast.show(t("cover.waitForUpload"));
    return;
  }
  coverUploadFor.value = detail.id;
  try {
    const item = await sendPersonalCover(entryId, file);
    if (item && book.value === detail) onCoverChanged(item, restore);
  } finally {
    coverUploadFor.value = null;
  }
}

function startClubAction(action: "pick" | "nominate") {
  const detail = book.value;
  if (!detail) return;
  clubActions.value?.start(action, { id: detail.id, title: detail.title, currentIn: detail.club_picks });
}

function onPicked(club: ClubRef) {
  const detail = book.value;
  if (detail && !detail.club_picks.some((row) => row.slug === club.slug)) {
    detail.club_picks = [...detail.club_picks, club];
  }
}

async function refreshDetails() {
  const detail = book.value;
  if (!detail || refreshing.value) return;
  refreshing.value = true;
  try {
    const fresh = await api.refreshBook(detail.id);
    book.value = fresh;
    syncDrafts(fresh);
    toast.show(t("book.detailsUpdated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.refreshFailed"));
  } finally {
    refreshing.value = false;
  }
}

/** The admin's values are saved: the answer is the book as it is now. */
function adoptAdminEdit(value: BookDetailOut | EditionOut) {
  if (!("readers" in value)) return;
  book.value = value;
  syncDrafts(value);
}

async function editEditionAsAdmin() {
  const editionId = book.value?.edition_id;
  if (editionId == null) return;
  try {
    adminEdition.value = await api.edition(editionId);
    adminEditing.value = "edition";
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("admin.edit.loadFailed"));
  }
}

/**
 * The admin set the book's own cover. The answer is the work's cover, but a
 * page in the viewer's edition shows the edition's cover when it has one, so
 * then the page reads the book again rather than show a cover its readers
 * do not see.
 */
function adminCoverSet(updated: BookOut) {
  if (book.value?.edition_id == null) adoptCover(updated);
  else void adminEditionSaved();
}

/** An edition changed: its title, pages or cover may be what the page shows. */
async function adminEditionSaved() {
  editionsVersion.value += 1;
  const detail = book.value;
  if (!detail) return;
  try {
    const fresh = await api.book(detail.id);
    book.value = fresh;
    syncDrafts(fresh);
  } catch {
    /* Cosmetic; the next load catches up. */
  }
}

/**
 * Close the "…" sheet before anything it asked for: a second sheet on top of
 * it would close with it on one Escape and trap Tab twice. Focus goes back to
 * "…", so the next sheet returns it there when it closes.
 */
async function closeSheets() {
  moreOpen.value = false;
  adminEditionCover.value = false;
  choosingCover.value = false;
  await nextTick();
  moreButton.value?.focus({ preventScroll: true });
}

async function onMoreAction(action: BookAction) {
  // The row shows "Refreshing…" until the answer is in; then the sheet goes.
  if (action === "refresh") {
    await refreshDetails();
    if (moreOpen.value) await closeSheets();
    return;
  }
  await closeSheets();
  if (action === "edit") requestingEdit.value = true;
  else if (action === "merge") requestingMerge.value = true;
  else if (action === "report-book") reportBook();
  else if (action === "report-cover") reportCover();
  else if (action === "admin-edit") adminEditing.value = "work";
  else if (action === "admin-edition") await editEditionAsAdmin();
  else if (action === "admin-edition-cover") adminEditionCover.value = true;
  else if (action === "choose-cover") {
    // A photo still uploading would land after this choice, with an Undo to the cover before both.
    if (coverUploading.value) toast.show(t("cover.waitForUpload"));
    else choosingCover.value = true;
  }
}

/**
 * A photo chosen in a sheet: the sheet closes at once and the page uploads
 * it, so the answer still reaches the page (a closed sheet's emits are lost).
 */
async function uploadPicked(target: CoverTarget, file: File, done: (updated: BookOut) => void) {
  await closeSheets();
  // One upload at a time; a second pick says so instead of vanishing.
  if (coverUploadFor.value !== null) {
    toast.show(t("cover.waitForUpload"));
    return;
  }
  coverUploadFor.value = target.workId;
  try {
    const updated = await sendCoverPhoto(target, file);
    if (updated && book.value?.id === target.workId) done(updated);
  } finally {
    coverUploadFor.value = null;
  }
}

function onMorePicked(kind: "photo" | "admin", file: File) {
  const detail = book.value;
  if (!detail) return;
  if (kind === "admin") void uploadPicked({ workId: detail.id, editionId: null, admin: true }, file, adminCoverSet);
  else void uploadPicked({ workId: detail.id, editionId: detail.edition_id }, file, adoptCover);
}

function onEditionCoverPicked(editionId: number, file: File) {
  const detail = book.value;
  if (detail) void uploadPicked({ workId: detail.id, editionId, admin: true }, file, () => void adminEditionSaved());
}

function reportCover() {
  const detail = book.value;
  if (detail?.cover_public_id) reporting.value = { kind: "cover", publicId: detail.cover_public_id, title: shownTitle.value };
}

/** Only a book a reader added (`custom`) can be reported, until its text is withdrawn; the others are corrected instead. */
function reportBook() {
  const detail = book.value;
  if (detail?.custom && !withdrawn.value) reporting.value = { kind: "work", workId: detail.id, title: detail.title };
}

async function toggleFavorite() {
  const detail = book.value;
  if (!detail || busy.value) return;
  busy.value = true;
  try {
    if (detail.favorite_position) {
      await api.removeFavorite(detail.id);
      detail.favorite_position = null;
      toast.show(t("favorites.removed"));
    } else {
      const result = await api.addFavorite(detail.id);
      const mine = result.items.find((row) => row.book.id === detail.id);
      detail.favorite_position = mine?.position ?? null;
      toast.show(t("favorites.added"));
    }
  } catch (err) {
    if (err instanceof ApiError && err.status === 409) {
      toast.show(t("favorites.full"));
    } else {
      toast.show(err instanceof ApiError ? err.message : t("favorites.saveFailed"));
    }
  } finally {
    busy.value = false;
  }
}

onMounted(load);
onUnmounted(() => {
  loadAbort?.abort();
  stopLoadClock();
});
watch(param, load);
</script>

<template>
  <section>
    <p class="fine back-link">
      <a :href="backPath" @click.prevent="goBack">{{ backLabel }}</a>
    </p>

    <template v-if="!loaded">
      <!-- The page's shape while it loads: the title, the status control and two
           lines of the panel, so nothing below jumps when the book arrives. -->
      <div class="detail-hero detail-skeleton" aria-hidden="true">
        <span class="skeleton hero-cover-skeleton" />
        <div class="detail-side">
          <div class="detail-intro">
            <span class="skeleton skeleton-line skeleton-title" />
            <span class="skeleton skeleton-line short" />
          </div>
          <div class="shelf-panel skeleton-panel">
            <span class="skeleton skeleton-status" />
            <span class="skeleton skeleton-line" />
            <span class="skeleton skeleton-line short" />
          </div>
        </div>
      </div>
      <p v-if="loadWaitSec >= 2" class="fine subtle" aria-live="polite">
        {{ t("book.stillLoading", { n: loadWaitSec }) }}
      </p>
    </template>

    <div v-else-if="error" class="empty">
      <h3>{{ t("book.loadFailedTitle") }}</h3>
      <p>{{ error }}</p>
      <div class="btn-row">
        <button v-if="retryable" class="btn btn-ghost" type="button" @click="load">
          {{ t("common.tryAgain") }}
        </button>
        <RouterLink class="btn btn-ghost" to="/discover">{{ t("book.backToDiscover") }}</RouterLink>
      </div>
    </div>

    <template v-else-if="book">
      <div class="detail-hero">
        <div class="hero-cover" :class="{ uploading: coverUploading }" :aria-busy="coverUploading">
          <BookCover :book="withdrawn ? { ...book, title: shownTitle } : book" eager />
          <p v-if="coverUploading" class="finer subtle cover-status" role="status">{{ t("cover.photoUploading") }}</p>
          <!-- The only call to action on a bare cloth; with a cover, the "…" sheet suggests a photo.
               Hidden, not removed, during a sheet's upload: its own upload must keep its listener. -->
          <CoverPhotoButton
            v-if="!coverPrompted && !hasCover"
            v-show="!coverUploading"
            :work-id="book.id"
            :edition-id="book.edition_id"
            :has-cover="false"
            @covered="adoptCover"
          />
        </div>
        <div class="detail-side">
          <div class="detail-intro">
            <div class="intro-head">
              <button
                ref="moreButton"
                class="icon-btn more-btn"
                type="button"
                aria-haspopup="dialog"
                :aria-expanded="moreOpen"
                :aria-label="t('book.moreActions')"
                @click="moreOpen = true"
              >
                <EllipsisHorizontalIcon class="more-icon" aria-hidden="true" />
              </button>
              <p v-if="book.club_picks.length" class="hero-pick">
                <span class="badge club-pick"><FlagIcon class="pick-flag" aria-hidden="true" />{{ t("book.pickBadge", { club: pickNames }) }}</span>
              </p>
              <h1 class="display" :class="{ 'long-title': longTitle }">{{ shownTitle }}</h1>
            </div>
            <p v-if="withdrawn" class="fine subtle">{{ t("report.removedNote") }}</p>
            <p v-if="book.subtitle" class="detail-subtitle">{{ book.subtitle }}</p>
            <p v-if="book.authors || book.year" class="detail-authors">
              <template v-for="(name, index) in authorNames" :key="index"><template v-if="index">, </template><span class="author-name">{{ name }}</span></template><span v-if="book.year" class="detail-year"><span v-if="book.authors" aria-hidden="true"> · </span><span
                class="nums"
                :title="t('book.firstPublished', { year: book.year })"
              >{{ book.year }}</span></span>
            </p>
            <p v-if="book.rating_count" class="club-rating">
              <span class="visually-hidden">{{ t("book.ratingLabel") }}</span>
              <span class="stars" aria-hidden="true">★</span>
              <strong class="nums">{{ formatRating(book.club_rating, locale as Locale) }}</strong>
              <span class="subtle">· {{ tp("book.ratingVotes", book.rating_count) }}</span>
            </p>
          </div>

          <CoverPrompt :book="book" class="detail-cover-prompt" @covered="adoptCover" />

          <div class="shelf-panel">
            <div class="segmented status-control" role="group" :aria-label="t('book.shelfStatus')">
              <button
                v-for="status in STATUSES"
                :key="status"
                type="button"
                :aria-pressed="book.on_shelf === status"
                :disabled="busy"
                @click="setStatus(status)"
              >
                {{ statusShort(status) }}
              </button>
            </div>

            <p v-if="!book.on_shelf" class="fine subtle">
              {{ t("book.notOnShelf") }}
            </p>

            <div v-if="book.on_shelf === 'currently_reading'" class="control-block">
              <ProgressEditor
                :value="book.progress"
                :page="book.current_page"
                :pages="book.progress_pages ?? book.pages"
                :label="t('book.progress')"
                :book-title="book.title"
                :busy="busy"
                :shelf-id="book.shelf_id"
                :club-slug="pickClubSlug"
                @save="saveProgress"
                @finished="onFinished"
              />
            </div>

            <!-- Light rows with hairlines between them: the stars, "only me", the line. -->
            <div v-if="book.on_shelf === 'finished'" class="finished-rows">
              <div class="light-row rating-row">
                <div class="rating-head">
                  <span class="fine">{{ t("book.yourRating") }}</span>
                  <button
                    v-if="book.rating"
                    class="text-btn clear-rating finer"
                    type="button"
                    :disabled="busy"
                    @click="clearRating"
                  >
                    {{ t("book.clearRating") }}
                  </button>
                </div>
                <div class="star-row" role="group" :aria-label="t('book.yourRating')">
                  <button
                    v-for="value in 5"
                    :key="value"
                    class="star-btn"
                    type="button"
                    :aria-label="t('book.starsOf', { n: value })"
                    :aria-pressed="book.rating === value"
                    :disabled="busy"
                    @click="rate(value)"
                  >
                    {{ book.rating && book.rating >= value ? "★" : "☆" }}
                  </button>
                </div>
              </div>
              <label class="light-row check-row">
                <span class="fine">{{ t("book.ratingOnlyMe") }}</span>
                <input
                  type="checkbox"
                  :checked="book.rating_visibility === 'only_me'"
                  :disabled="busy"
                  @change="setRatingVisibility"
                />
              </label>
              <div class="light-row take-row">
                <label class="field">
                  <span class="field-head">{{ t("book.oneLineTake") }}<span class="finer subtle nums">{{ takeDraft.length }}/140</span></span>
                  <textarea
                    v-model="takeDraft"
                    v-autosize
                    class="short-note"
                    rows="2"
                    maxlength="140"
                    :placeholder="t('book.takePlaceholder')"
                  />
                </label>
                <button
                  v-if="takeDraft !== book.take"
                  class="btn btn-ghost btn-sm"
                  type="button"
                  :disabled="busy"
                  @click="patch({ take: takeDraft }, t('book.takeSaved'))"
                >
                  {{ t("book.saveTake") }}
                </button>
              </div>
            </div>

            <div v-if="book.on_shelf === 'did_not_finish'" class="control-block">
              <label class="field">
                <span class="field-head">{{ t("book.dnfWhy") }}<span class="finer subtle nums">{{ reasonDraft.length }}/200</span></span>
                <textarea
                  v-model="reasonDraft"
                  v-autosize
                  class="short-note"
                  rows="2"
                  maxlength="200"
                  :placeholder="t('common.optional')"
                />
              </label>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busy || reasonDraft === book.dnf_reason"
                @click="patch({ dnf_reason: reasonDraft }, t('book.reasonSaved'))"
              >
                {{ t("book.saveReason") }}
              </button>
            </div>

            <EditionPicker
              v-if="book.shelf_id"
              :key="editionsVersion"
              :work-id="book.id"
              :edition-id="book.edition_id"
              :disabled="busy"
              @choose="(editionId) => patch({ edition_id: editionId }, t('editions.saved'))"
            />

            <!-- The book's other doings with this reader: light rows, and Remove last and quiet. -->
            <ul class="sheet-rows panel-actions">
              <li>
                <button
                  class="sheet-row favorite-row"
                  type="button"
                  :aria-pressed="Boolean(book.favorite_position)"
                  :disabled="busy"
                  @click="toggleFavorite"
                >
                  <component
                    :is="book.favorite_position ? HeartSolidIcon : HeartIcon"
                    class="sheet-icon"
                    aria-hidden="true"
                  />
                  <span class="sheet-label">{{
                    book.favorite_position
                      ? t("favorites.markedAt", { n: book.favorite_position })
                      : t("favorites.mark")
                  }}</span>
                </button>
              </li>
              <li v-if="pickOpen.length">
                <button class="sheet-row pick-row" type="button" @click="startClubAction('pick')">
                  <FlagIcon class="sheet-icon" aria-hidden="true" />
                  <span class="sheet-label">{{ pickRowLabel }}</span>
                </button>
              </li>
              <li v-if="canNominate">
                <button class="sheet-row" type="button" @click="startClubAction('nominate')">
                  <HandRaisedIcon class="sheet-icon" aria-hidden="true" />
                  <span class="sheet-label">{{ t("book.nominate") }}</span>
                </button>
              </li>
            </ul>
            <button
              v-if="book.on_shelf"
              class="text-btn danger remove-btn"
              type="button"
              :disabled="busy"
              @click="remove"
            >
              {{ t("common.remove") }}
            </button>
          </div>
        </div>
      </div>

      <div class="book-body">
        <section v-if="book.description" class="section">
          <div class="section-head">
            <h2>{{ t("book.about") }}</h2>
          </div>
          <p class="description" :class="{ clamped: longDescription && !expanded }">
            {{ book.description }}
          </p>
          <button
            v-if="longDescription"
            class="text-btn"
            type="button"
            @click="expanded = !expanded"
          >
            {{ expanded ? t("book.showLess") : t("book.readMore") }}
          </button>
        </section>

        <section class="section">
          <BookDiary ref="diary" :work-id="book.id" />
        </section>

        <section class="section">
          <QuoteList :work-id="book.id" :empty-text="t('quotes.emptyBook')" />
        </section>

        <section v-if="book.subjects.length" class="section">
          <div class="section-head">
            <h2>{{ t("book.subjects") }}</h2>
          </div>
          <div class="chip-row">
            <RouterLink
              v-for="subject in book.subjects"
              :key="subject"
              class="chip"
              :to="{ path: '/discover', query: { q: subject } }"
            >
              {{ subject }}
            </RouterLink>
          </div>
        </section>

        <section class="section">
          <div class="section-head">
            <h2>{{ t("book.otherReaders") }}</h2>
          </div>
          <ul v-if="book.readers.length" class="reader-list">
            <li v-for="reader in book.readers" :key="reader.username">
              <RouterLink class="reader-row" :to="`/u/${reader.username}`">
                <Avatar :username="reader.username" :src="reader.avatar_url" size="sm" />
                <span class="reader-name">{{ reader.username }}</span>
                <span class="badge" :class="reader.status">
                  {{ statusShort(reader.status) }}
                </span>
                <span v-if="reader.progress != null" class="fine subtle nums">
                  {{ t("progress.percent", { n: reader.progress }) }}
                </span>
                <span v-if="reader.rating" class="fine stars">
                  {{ starLabel(reader.rating) }}
                </span>
              </RouterLink>
            </li>
          </ul>
          <p v-else class="fine subtle">{{ t("book.nobodyElse") }}</p>
        </section>
      </div>
    </template>

    <ClubBookActions ref="clubActions" @picked="onPicked" />
    <MergeRequestSheet
      v-if="requestingMerge && book"
      :work-id="book.id"
      :title="shownTitle"
      @close="requestingMerge = false"
    />
    <EditRequestSheet v-if="requestingEdit && book" :book="book" @close="requestingEdit = false" />
    <EditRequestSheet
      v-if="adminEditing === 'work' && book"
      admin
      :book="book"
      @saved="adoptAdminEdit"
      @close="adminEditing = null"
    />
    <EditRequestSheet
      v-if="adminEditing === 'edition' && adminEdition"
      admin
      :edition="adminEdition"
      @saved="adminEditionSaved"
      @close="adminEditing = null"
    />
    <ReportSheet v-if="reporting" :target="reporting" @close="reporting = null" />
    <BookActionsSheet
      v-if="moreOpen && book"
      :book="book"
      :title="shownTitle"
      :admin="Boolean(session.user?.is_admin)"
      :cover-prompted="coverPrompted"
      :withdrawn="withdrawn"
      :refreshing="refreshing"
      @act="onMoreAction"
      @picked="onMorePicked"
      @close="moreOpen = false"
    />
    <CoverChooserSheet
      v-if="choosingCover && book?.shelf_id != null"
      :entry-id="book.shelf_id"
      :work-id="book.id"
      :title="shownTitle"
      :personal-cover="Boolean(book.personal_cover)"
      :cover-public-id="book.cover_public_id"
      @changed="onCoverChanged"
      @picked="onPersonalPhoto"
      @close="closeSheets"
    />
    <AdminEditionCover
      v-if="adminEditionCover && book && session.user?.is_admin"
      :work-id="book.id"
      :edition-id="book.edition_id"
      :title="shownTitle"
      @picked="onEditionCoverPicked"
      @close="adminEditionCover = false"
    />
  </section>
</template>

<style scoped>
.back-link {
  margin-bottom: var(--space-4);
}

.detail-hero {
  display: grid;
  grid-template-columns: 120px 1fr;
  gap: var(--space-4);
  align-items: start;
  margin-bottom: var(--space-5);
}

/* Narrow screens put the intro beside the cover and the shelf panel on its own
   full-width row, so the wrapper dissolves into the hero grid. */
.detail-side {
  display: contents;
}

.detail-hero .shelf-panel,
.detail-hero .detail-cover-prompt {
  grid-column: 1 / -1;
}

.hero-cover-skeleton {
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-md);
}

/* Tall enough that the footer does not jump when the book arrives. */
.detail-skeleton {
  min-height: 60vh;
  align-content: start;
}

.detail-skeleton .detail-intro {
  gap: var(--space-2);
}

.skeleton-title {
  height: 1.6em;
  width: 70%;
}

.skeleton-status {
  height: 46px;
  border-radius: var(--radius-pill);
}

.hero-cover :deep(.cover-photo),
.cover-status {
  display: block;
  margin-top: var(--space-1);
  color: var(--text-subtle);
}

.cover-status {
  display: flex;
  align-items: center;
  min-height: var(--tap);
}

/* A photo from the "…" sheet is on its way: the sheet is gone, the cover says so. */
.hero-cover.uploading :deep(.cover) {
  opacity: 0.6;
  transition: opacity var(--dur-base) var(--ease-out);
}

.check-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--tap);
}

/* The finished book's own rows: hairlines between them, no box around each. */
.finished-rows {
  display: grid;
  border-top: 1px solid var(--border);
}

.light-row {
  padding: var(--space-2) 0;
  border-bottom: 1px solid var(--border);
}

.light-row:last-child {
  border-bottom: 0;
  padding-bottom: 0;
}

.rating-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-height: var(--tap);
}

.clear-rating {
  margin-right: calc(-1 * var(--space-2));
  color: var(--text-muted);
  font-weight: 400;
}

/* The stars line up with the text above them; each keeps its 44 px target. */
.rating-row .star-row {
  margin-left: calc(-1 * var(--space-2));
}

.finished-rows .check-row {
  justify-content: space-between;
  color: var(--text-muted);
}

.finished-rows .check-row input {
  width: 20px;
  height: 20px;
  accent-color: var(--accent);
}

.take-row {
  display: grid;
  gap: var(--space-2);
}

.take-row .field {
  margin-bottom: 0;
}

.take-row .btn {
  justify-self: start;
}

.detail-intro {
  min-width: 0;
  display: grid;
  gap: var(--space-1);
  align-content: start;
}

.detail-intro .display {
  font-size: var(--text-2xl);
  text-wrap: balance;
}

/* A very long title steps down a size, so the status control stays in view. */
.detail-intro .display.long-title {
  font-size: var(--text-xl);
}

/* "…" floats beside the title's first lines; the rest of the title takes the full width.
   The negative margins line the icon up with the text while the target stays 44 px. */
.more-btn {
  float: inline-end;
  margin: calc(-1 * var(--space-2)) calc(-1 * var(--space-2)) 0 var(--space-1);
}

.more-icon {
  width: 24px;
  height: 24px;
}

.hero-pick {
  margin: 0 0 var(--space-1);
  line-height: 1;
}

/* Every club's name stays readable: the badge wraps rather than cutting. The
   flag is the one on the "set as pick" row: this is that row's done state. */
.hero-pick .badge {
  max-width: 100%;
  white-space: normal;
  line-height: 1.3;
}

.pick-flag {
  flex: none;
  width: 0.95em;
  height: 0.95em;
  margin-right: 0.3em;
  vertical-align: -0.12em;
  stroke-width: 2;
}

.detail-subtitle {
  margin-top: calc(-1 * var(--space-1));
  font-family: var(--serif);
  font-size: var(--text-lg);
}

.detail-authors {
  color: var(--text-muted);
  font-size: var(--text-lg);
}

/* "· 2019" never starts a line of its own, and a name never breaks inside itself. */
.detail-year,
.author-name {
  white-space: nowrap;
}

.club-rating {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1);
  font-size: var(--text-sm);
}

.shelf-panel {
  display: grid;
  gap: var(--space-3);
}

/* Equal segments where the words fit; a longer word ("Abgebrochen") takes
   its own width instead of running out of its segment. */
.status-control button {
  flex: 1 1 0;
  min-width: max-content;
}

/* Four words need no 690 px: on a wide panel the segment keeps a hand's width. */
@media (min-width: 720px) {
  .status-control {
    max-width: 30rem;
  }
}

.control-block {
  display: grid;
  gap: var(--space-2);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

.control-block .field {
  margin-bottom: 0;
}

.field-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.field-head .finer {
  font-weight: 400;
}

.field textarea.short-note {
  min-height: 4.25rem;
}

.control-block .btn {
  justify-self: start;
}

/* The panel's own rows: hairlines, no boxes; the text lines up with the column. */
.panel-actions {
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

.panel-actions .sheet-row {
  min-height: 48px;
  padding: 0;
  font-weight: 500;
}

.favorite-row[aria-pressed="true"] .sheet-icon {
  color: var(--accent);
}

.remove-btn {
  justify-self: start;
  margin: calc(-1 * var(--space-2)) 0 0 calc(-1 * var(--space-2));
  font-size: var(--text-sm);
  font-weight: 500;
}

.description {
  white-space: pre-line;
  color: var(--text-muted);
  max-width: 68ch;
}

.description.clamped {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 6;
  line-clamp: 6;
  overflow: hidden;
}

/* Light rows; the whole row opens the reader's profile. */
.reader-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.reader-list li + li {
  border-top: 1px solid var(--border);
}

.reader-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: 56px;
  padding: var(--space-2) 0;
  color: inherit;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.reader-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.reader-name {
  min-width: 0;
  margin-right: auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

@media (hover: hover) {
  .reader-row:hover .reader-name {
    color: var(--accent);
  }
}

@media (min-width: 720px) {
  .detail-hero {
    grid-template-columns: 200px 1fr;
    gap: var(--space-6);
  }

  .detail-intro .display {
    font-size: var(--text-3xl);
  }

  .detail-intro .display.long-title {
    font-size: var(--text-2xl);
  }
}

@media (min-width: 1024px) {
  .detail-hero {
    grid-template-columns: 240px minmax(0, 720px);
  }

  /* Wide enough to stack the shelf panel under the intro beside the cover. */
  .detail-side {
    display: grid;
    gap: var(--space-5);
    align-content: start;
    min-width: 0;
  }

  .detail-hero .shelf-panel,
  .detail-hero .detail-cover-prompt {
    grid-column: auto;
  }

  /* The sections below keep to the column beside the cover, so every right
     edge on the page (panel, diary, readers) is the same one. */
  .book-body {
    max-width: 720px;
    margin-inline-start: calc(240px + var(--space-6));
  }
}
</style>
