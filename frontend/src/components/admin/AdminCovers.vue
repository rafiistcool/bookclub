<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { AdminCoverSummary, CoverSuggestionOut, MissingCoverOut } from "../../api/generated";
import {
  coverCandidateUrl,
  coverFound,
  coverReasonLabel,
  coverSearchMessage,
  coverStatusLabel,
  missingCoverNotes,
} from "../../admin";
import { bookPath } from "../../constants";
import { formatClubDate, formatDay } from "../../i18n/dates";
import { languageName } from "../../languages";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";
import CoverPhotoButton from "../CoverPhotoButton.vue";
import { useQueue } from "./useQueue";

/**
 * The cover search's work for an admin (#74). "Suggestions": covers the
 * search found but did not attach on its own, our book beside Open Library's,
 * with the reason; "Use this cover" attaches the picture, "Not this book"
 * keeps that Open Library work out for good (both after a second tap).
 * "Without a cover": the catalog and imported books that still have none (or
 * whose Open Library picture fails to load), each with how the search went,
 * "Search now", a direct upload and the way to the book. Readers' own books
 * are not listed: they may keep their cloth.
 */
const props = defineProps<{
  /** `AdminSummary.covers`, once loaded: the counts on the two views. */
  counts: AdminCoverSummary | null;
}>();
const emit = defineEmits<{ changed: [] }>();

type Section = "suggestions" | "missing";
type Scope = "in_use" | "all";
/** A book of the missing list, keyed by its work id for the queue. */
type MissingRow = MissingCoverOut & { id: number };

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const section = ref<Section>("suggestions");

const suggestions = useQueue<CoverSuggestionOut>((view, page) => api.adminCoverSuggestionPage(view, page));
const missing = useQueue<MissingRow, Scope>(async (scope, page) => {
  const answer = await api.adminMissingCovers(scope, page);
  return { ...answer, items: answer.items.map((item) => ({ ...item, id: item.book.id })) };
}, "in_use");

/** A decision needs a second tap: which suggestion, and which decision. */
const confirming = ref<{ id: number; action: "approve" | "reject" } | null>(null);
const busyId = ref<number | null>(null);
const searchingId = ref<number | null>(null);
/** A decision or a search changed the other view: read it again when it is shown. */
let suggestionsStale = false;
let missingStale = true;

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

function zone(): string {
  return session.user?.timezone || "UTC";
}

function when(iso: string | null): string {
  return formatClubDate(iso, zone(), locale.locale);
}

function day(iso: string): string {
  return formatDay(iso, zone(), locale.locale);
}

async function showSection(next: Section) {
  if (section.value === next) return;
  section.value = next;
  confirming.value = null;
  if (next === "missing" && missingStale) {
    missingStale = false;
    await missing.load();
  } else if (next === "suggestions" && suggestionsStale) {
    suggestionsStale = false;
    await suggestions.load();
  }
}

function showView(next: "open" | "resolved") {
  confirming.value = null;
  void suggestions.show(next);
}

async function decide(suggestion: CoverSuggestionOut, action: "approve" | "reject") {
  if (busyId.value) return;
  if (confirming.value?.id !== suggestion.id || confirming.value.action !== action) {
    confirming.value = { id: suggestion.id, action };
    return;
  }
  confirming.value = null;
  busyId.value = suggestion.id;
  try {
    if (action === "approve") {
      await api.approveCoverSuggestion(suggestion.id);
      toast.show(t("admin.covers.used"));
    } else {
      await api.rejectCoverSuggestion(suggestion.id);
      toast.show(t("admin.covers.rejected"));
    }
    suggestions.drop((row) => row.id === suggestion.id);
    missingStale = true;
    emit("changed");
  } catch (err) {
    // Decided meanwhile (409), the book has a cover now (409), the picture is
    // gone (400) or the suggestion is (404): the list read again shows what is left.
    if (err instanceof ApiError && [400, 404, 409].includes(err.status)) {
      missingStale = true;
      emit("changed");
      await suggestions.load();
    }
    toast.show(failed(err, t("admin.decideFailed")));
  } finally {
    busyId.value = null;
  }
}

function hasPicture(suggestion: CoverSuggestionOut): boolean {
  return Boolean(suggestion.cover_url || suggestion.cover_thumb_url);
}

/** Open Library's year and language of the candidate, as one line. */
function candidateFacts(suggestion: CoverSuggestionOut): string {
  const parts: string[] = [];
  if (suggestion.year != null) parts.push(String(suggestion.year));
  if (suggestion.language) parts.push(languageName(suggestion.language, locale.locale));
  return parts.join(" · ");
}

function decision(suggestion: CoverSuggestionOut): string {
  const key = ["approved", "rejected", "closed"].includes(suggestion.status) ? suggestion.status : "closed";
  return t(`admin.covers.decided.${key}`, { when: when(suggestion.decided_at) });
}

/** The book has a cover now: it leaves the list, and the counts changed. */
function covered(row: MissingRow) {
  missing.drop((item) => item.id === row.id);
  suggestionsStale = true;
  emit("changed");
}

async function searchNow(row: MissingRow) {
  if (searchingId.value) return;
  searchingId.value = row.id;
  try {
    const answer = await api.adminSearchCover(row.id);
    toast.show(coverSearchMessage(answer.outcome));
    if (coverFound(answer.outcome)) {
      covered(row);
    } else if (answer.outcome === "suggested") {
      row.suggestion_id = answer.suggestion?.id ?? row.suggestion_id;
      suggestionsStale = true;
      emit("changed");
    } else if (answer.outcome === "failing") {
      // Fetched again and still not loading: the row stays, with the new date.
      row.state = "failing";
      row.failed_at = answer.failed_at ?? row.failed_at;
      row.failing_edition_id = answer.failing_edition_id ?? null;
    } else {
      row.outcome = "no_match";
      row.searched_at = new Date().toISOString();
      row.next_search_at = null;
      row.failures = 0;
    }
  } catch (err) {
    // 409: it has a cover that loads now; 404: no longer a catalog book here.
    if (err instanceof ApiError && (err.status === 409 || err.status === 404)) covered(row);
    toast.show(failed(err, t("admin.covers.searchFailed")));
  } finally {
    searchingId.value = null;
  }
}

function notes(row: MissingRow): string[] {
  return missingCoverNotes(row, day);
}

function uploadLabel(row: MissingRow): string {
  return row.failing_edition_id != null
    ? t("admin.covers.uploadEdition", { id: row.failing_edition_id })
    : t("admin.covers.upload");
}

function count(value: number | undefined): number {
  return value ?? 0;
}

onMounted(() => suggestions.load());
</script>

<template>
  <section class="admin-queue admin-covers" aria-labelledby="admin-covers">
    <div class="section-head">
      <h2 id="admin-covers">{{ t("admin.covers.title") }}</h2>
    </div>
    <p class="fine muted">{{ t("admin.covers.blurb") }}</p>

    <div class="segmented cover-sections" role="group" :aria-label="t('admin.covers.views')">
      <button type="button" :aria-pressed="section === 'suggestions'" @click="showSection('suggestions')">
        {{ t("admin.covers.suggestions") }}
        <span v-if="count(props.counts?.suggestions)" class="seg-count nums">{{ props.counts?.suggestions }}</span>
      </button>
      <button type="button" :aria-pressed="section === 'missing'" @click="showSection('missing')">
        {{ t("admin.covers.missing") }}
        <span v-if="count(props.counts?.missing)" class="seg-count nums">{{ props.counts?.missing }}</span>
      </button>
    </div>

    <template v-if="section === 'suggestions'">
      <p class="fine subtle">{{ t("admin.covers.suggestionsBlurb") }}</p>
      <div class="segmented queue-views" role="group" :aria-label="t('admin.views')">
        <button type="button" :aria-pressed="suggestions.view.value === 'open'" @click="showView('open')">
          {{ t("admin.openView") }}
        </button>
        <button type="button" :aria-pressed="suggestions.view.value === 'resolved'" @click="showView('resolved')">
          {{ t("admin.history") }}
        </button>
      </div>

      <p v-if="suggestions.error.value" class="error">
        {{ suggestions.error.value }}
        <button class="text-btn" type="button" @click="suggestions.load">{{ t("common.tryAgain") }}</button>
      </p>
      <div v-else-if="!suggestions.loaded.value" class="skeleton admin-skeleton" aria-hidden="true" />
      <p v-else-if="suggestions.items.value.length === 0" class="fine subtle">
        {{ suggestions.view.value === "open" ? t("admin.covers.noSuggestions") : t("admin.noHistory") }}
      </p>
      <ol v-else class="admin-list">
        <li v-for="suggestion in suggestions.items.value" :key="suggestion.id" class="panel admin-item suggestion-item">
          <p class="admin-item-head fine">
            <span v-if="suggestions.view.value === 'resolved'" class="badge" :class="`status-${suggestion.status}`">
              {{ coverStatusLabel(suggestion.status) }}
            </span>
            <span class="subtle">{{ t("admin.covers.found", { when: when(suggestion.created_at) }) }}</span>
          </p>
          <p class="fine suggestion-reason">{{ coverReasonLabel(suggestion.reason) }}</p>

          <div class="cover-compare">
            <figure class="compare-side">
              <figcaption class="finer subtle">{{ t("admin.covers.ourBook") }}</figcaption>
              <RouterLink v-if="suggestion.book" :to="bookPath(suggestion.book)" class="compare-book">
                <BookCover :book="suggestion.book" size="md" />
                <span class="compare-text">
                  <strong>{{ suggestion.book.title }}</strong>
                  <span v-if="suggestion.book.authors" class="fine subtle">{{ suggestion.book.authors }}</span>
                </span>
              </RouterLink>
              <p v-else class="fine subtle">{{ t("admin.bookGone") }}</p>
            </figure>
            <figure class="compare-side candidate">
              <figcaption class="finer subtle">{{ t("admin.covers.candidate") }}</figcaption>
              <div class="compare-book">
                <BookCover
                  v-if="hasPicture(suggestion)"
                  :book="{ title: suggestion.title, cover_url: suggestion.cover_url, cover_thumb_url: suggestion.cover_thumb_url }"
                  size="lg"
                />
                <span v-else class="finer subtle no-picture">{{ t("admin.covers.noCandidatePicture") }}</span>
                <span class="compare-text">
                  <strong>{{ suggestion.title }}</strong>
                  <span v-if="suggestion.authors" class="fine subtle">{{ suggestion.authors }}</span>
                  <span v-if="candidateFacts(suggestion)" class="finer subtle">{{ candidateFacts(suggestion) }}</span>
                  <a
                    v-if="coverCandidateUrl(suggestion)"
                    class="fine"
                    :href="coverCandidateUrl(suggestion)!"
                    target="_blank"
                    rel="noreferrer"
                  >
                    {{ t("admin.covers.viewOpenLibrary") }}
                  </a>
                </span>
              </div>
            </figure>
          </div>

          <p v-if="suggestions.view.value === 'resolved'" class="fine subtle decision">{{ decision(suggestion) }}</p>
          <template v-else>
            <p v-if="confirming?.id === suggestion.id" class="fine confirm-blurb">
              {{ confirming?.action === "approve" ? t("admin.covers.useBlurb") : t("admin.covers.rejectBlurb") }}
            </p>
            <div class="btn-row">
              <button
                class="btn btn-primary btn-sm"
                type="button"
                :disabled="busyId === suggestion.id || !hasPicture(suggestion)"
                @click="decide(suggestion, 'approve')"
              >
                {{
                  confirming?.id === suggestion.id && confirming?.action === "approve"
                    ? t("admin.covers.useConfirm")
                    : t("admin.covers.use")
                }}
              </button>
              <button
                class="btn btn-danger btn-sm"
                type="button"
                :disabled="busyId === suggestion.id"
                @click="decide(suggestion, 'reject')"
              >
                {{
                  confirming?.id === suggestion.id && confirming?.action === "reject"
                    ? t("admin.covers.rejectConfirm")
                    : t("admin.covers.reject")
                }}
              </button>
              <button
                v-if="confirming?.id === suggestion.id"
                class="btn btn-ghost btn-sm"
                type="button"
                @click="confirming = null"
              >
                {{ t("common.cancel") }}
              </button>
            </div>
          </template>
        </li>
      </ol>
      <button
        v-if="suggestions.loaded.value && suggestions.hasMore.value"
        class="btn btn-ghost btn-sm load-more"
        type="button"
        :disabled="suggestions.loadingMore.value"
        @click="suggestions.more"
      >
        {{ t("admin.loadMore") }}
      </button>
    </template>

    <template v-else>
      <p class="fine subtle">{{ t("admin.covers.missingBlurb") }}</p>
      <div class="segmented queue-views" role="group" :aria-label="t('admin.covers.scope')">
        <button type="button" :aria-pressed="missing.view.value === 'in_use'" @click="missing.show('in_use')">
          {{ t("admin.covers.scopeInUse") }}
        </button>
        <button type="button" :aria-pressed="missing.view.value === 'all'" @click="missing.show('all')">
          {{ t("admin.covers.scopeAll") }}
        </button>
      </div>

      <p v-if="missing.error.value" class="error">
        {{ missing.error.value }}
        <button class="text-btn" type="button" @click="missing.load">{{ t("common.tryAgain") }}</button>
      </p>
      <div v-else-if="!missing.loaded.value" class="skeleton admin-skeleton" aria-hidden="true" />
      <p v-else-if="missing.items.value.length === 0" class="fine subtle">{{ t("admin.covers.noMissing") }}</p>
      <ol v-else class="admin-list">
        <li v-for="row in missing.items.value" :key="row.id" class="panel admin-item missing-item">
          <div class="missing-book">
            <BookCover :book="row.book" size="sm" />
            <span class="compare-text">
              <strong>{{ row.book.title }}</strong>
              <span v-if="row.book.authors" class="fine subtle">{{ row.book.authors }}</span>
              <span v-if="row.in_use || row.imported" class="missing-badges">
                <span v-if="row.in_use" class="badge">{{ t("admin.covers.inUse") }}</span>
                <span v-if="row.imported" class="badge">{{ t("admin.covers.imported") }}</span>
              </span>
            </span>
          </div>
          <ul class="cover-notes fine">
            <li v-for="note in notes(row)" :key="note">{{ note }}</li>
          </ul>
          <div class="btn-row">
            <button
              class="btn btn-primary btn-sm"
              type="button"
              :disabled="searchingId === row.id"
              @click="searchNow(row)"
            >
              {{ searchingId === row.id ? t("admin.covers.searching") : t("admin.covers.searchNow") }}
            </button>
            <CoverPhotoButton
              admin
              look="ghost"
              :label="uploadLabel(row)"
              :work-id="row.id"
              :edition-id="row.failing_edition_id"
              :has-cover="false"
              @covered="covered(row)"
            />
            <RouterLink class="btn btn-ghost btn-sm" :to="bookPath(row.book)">{{ t("admin.covers.openBook") }}</RouterLink>
          </div>
        </li>
      </ol>
      <button
        v-if="missing.loaded.value && missing.hasMore.value"
        class="btn btn-ghost btn-sm load-more"
        type="button"
        :disabled="missing.loadingMore.value"
        @click="missing.more"
      >
        {{ t("admin.loadMore") }}
      </button>
    </template>
  </section>
</template>

<style scoped>
.cover-sections {
  max-width: 26rem;
  margin: var(--space-3) 0;
}

.queue-views {
  max-width: 22rem;
  margin: var(--space-3) 0;
}

.admin-skeleton {
  height: 160px;
}

.admin-list {
  list-style: none;
  margin: var(--space-3) 0 0;
  padding: 0;
  display: grid;
  gap: var(--space-3);
}

.admin-item {
  display: grid;
  gap: var(--space-3);
  min-width: 0;
}

.admin-item-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}

.badge.status-approved {
  color: var(--status-finished-fg);
}

.badge.status-rejected {
  color: var(--danger);
}

.suggestion-reason,
.confirm-blurb,
.decision {
  margin: 0;
}

/* Our book beside the candidate, in two equal columns even on a phone, and close together on a wide screen. */
.cover-compare {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-4);
  max-width: 34rem;
}

.compare-side {
  display: grid;
  align-content: start;
  gap: var(--space-2);
  min-width: 0;
  margin: 0;
}

.compare-side figcaption {
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-weight: 700;
}

.compare-book {
  display: grid;
  justify-items: start;
  gap: var(--space-2);
  min-width: 0;
  color: inherit;
  text-decoration: none;
}

.compare-text {
  display: grid;
  gap: 2px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.no-picture {
  display: grid;
  place-items: center;
  width: 104px;
  aspect-ratio: 2 / 3;
  padding: var(--space-2);
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-md);
  text-align: center;
}

.missing-book {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  min-width: 0;
}

.missing-badges {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  margin-top: var(--space-1);
}

.cover-notes {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
  color: var(--text-muted);
}

.load-more {
  margin-top: var(--space-3);
}
</style>
