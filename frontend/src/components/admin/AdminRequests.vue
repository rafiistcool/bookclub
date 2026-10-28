<script setup lang="ts">
import { vAutosize } from "../../autosize";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { BookDetailOut, EditionOut, EditRequestOut } from "../../api/generated";
import { mergeTarget, proposedFields, requestKindLabel } from "../../admin";
import { bookPath } from "../../constants";
import { formatClubDate } from "../../i18n/dates";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";
import EditRequestSheet from "../EditRequestSheet.vue";

/**
 * The catalog queue: corrections of books and editions, cover photos that
 * would replace a cover, and "these are the same book". A correction shows the
 * record's value now beside each proposed one. Approving applies the request
 * (an admin's values, which no catalog source overwrites); rejecting closes it
 * with an optional note. "Edit as admin" opens the request's edition, or else
 * its book, in the admin's own form; the request stays open for a decision.
 */
const emit = defineEmits<{ count: [n: number] }>();

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const items = ref<EditRequestOut[]>([]);
const loaded = ref(false);
const error = ref("");
const busyId = ref<number | null>(null);
const rejecting = ref<number | null>(null);
const rejectNote = ref("");
/**
 * Books the answer does not carry, by work id: a merge's target. Null when
 * the work is gone.
 */
const extraBooks = ref<Record<number, BookDetailOut | null>>({});
/** The record the admin edits directly, read fresh when the sheet opens. */
const editing = ref<{ book?: BookDetailOut; edition?: EditionOut } | null>(null);
const opening = ref<number | null>(null);

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

/** Reads the merge targets the queue names but does not carry, one request each. */
async function loadExtraBooks() {
  const ids = new Set<number>();
  for (const request of items.value) {
    const target = mergeTarget(request);
    if (target != null) ids.add(target);
  }
  await Promise.all(
    [...ids].filter((id) => !(id in extraBooks.value)).map(async (id) => {
      try {
        extraBooks.value[id] = await api.book(id);
      } catch {
        extraBooks.value[id] = null;
      }
    }),
  );
}

async function load() {
  try {
    items.value = (await api.adminEditRequests()).items;
    error.value = "";
    emit("count", items.value.length);
    void loadExtraBooks();
  } catch (err) {
    error.value = failed(err, t("admin.loadFailed"));
  } finally {
    loaded.value = true;
  }
}

function drop(id: number) {
  items.value = items.value.filter((row) => row.id !== id);
  emit("count", items.value.length);
}

/** Someone decided it meanwhile (409): it leaves the open queue either way. */
function decidedElsewhere(err: unknown): boolean {
  return err instanceof ApiError && err.status === 409;
}

async function approve(request: EditRequestOut) {
  if (busyId.value) return;
  busyId.value = request.id;
  try {
    await api.approveEditRequest(request.id);
    drop(request.id);
    toast.show(t("admin.approved"));
  } catch (err) {
    if (decidedElsewhere(err)) drop(request.id);
    toast.show(failed(err, t("admin.decideFailed")));
  } finally {
    busyId.value = null;
  }
}

function startReject(request: EditRequestOut) {
  rejecting.value = request.id;
  rejectNote.value = "";
}

async function reject(request: EditRequestOut) {
  if (busyId.value) return;
  busyId.value = request.id;
  try {
    await api.rejectEditRequest(request.id, rejectNote.value.trim());
    drop(request.id);
    rejecting.value = null;
    toast.show(t("admin.rejected"));
  } catch (err) {
    if (decidedElsewhere(err)) drop(request.id);
    toast.show(failed(err, t("admin.decideFailed")));
  } finally {
    busyId.value = null;
  }
}

/** "Edit as admin": the request's edition when it names one, else its book. */
async function editAsAdmin(request: EditRequestOut) {
  if (opening.value) return;
  opening.value = request.id;
  try {
    if (request.edition_id != null) {
      editing.value = { edition: await api.edition(request.edition_id) };
    } else if (request.work_id != null) {
      editing.value = { book: await api.book(request.work_id) };
    }
  } catch (err) {
    toast.show(failed(err, t("admin.edit.loadFailed")));
  } finally {
    opening.value = null;
  }
}

/** The record changed: read the queue again for the current values. */
function edited() {
  editing.value = null;
  void load();
}

function when(iso: string): string {
  return formatClubDate(iso, session.user?.timezone || "UTC", locale.locale);
}

onMounted(load);
</script>

<template>
  <section class="admin-queue" aria-labelledby="admin-requests">
    <div class="section-head">
      <h2 id="admin-requests">{{ t("admin.requests") }}</h2>
      <span v-if="items.length" class="fine subtle nums">{{ items.length }}</span>
    </div>
    <p class="fine muted">{{ t("admin.requestsBlurb") }}</p>
    <p v-if="error" class="error">
      {{ error }}
      <button class="text-btn" type="button" @click="load">{{ t("common.tryAgain") }}</button>
    </p>
    <div v-else-if="!loaded" class="skeleton admin-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0" class="fine subtle">{{ t("admin.noRequests") }}</p>
    <ol v-else class="admin-list">
      <li v-for="request in items" :key="request.id" class="panel admin-item request-item">
        <p class="admin-item-head fine">
          <span class="badge">{{ requestKindLabel(request.kind) }}</span>
          <span class="subtle">
            {{ t("admin.byOn", { name: request.created_by?.username ?? t("admin.someone"), when: when(request.created_at) }) }}
          </span>
        </p>

        <div class="admin-book">
          <RouterLink v-if="request.book" :to="bookPath(request.book)" class="admin-book-link">
            <BookCover :book="request.book" size="sm" />
            <span>
              <strong>{{ request.book.title }}</strong>
              <span v-if="request.book.authors" class="fine subtle">{{ request.book.authors }}</span>
              <span v-if="request.edition_id" class="finer subtle">{{ t("admin.edition", { id: request.edition_id }) }}</span>
            </span>
          </RouterLink>
          <p v-else class="fine subtle">{{ t("admin.bookGone") }}</p>

          <figure v-if="request.kind === 'cover_replace'" class="proposed-cover">
            <BookCover
              v-if="request.cover_url || request.cover_thumb_url"
              :book="{ title: request.book?.title ?? '', cover_url: request.cover_url, cover_thumb_url: request.cover_thumb_url }"
              size="sm"
            />
            <figcaption class="finer subtle">
              {{ request.cover_url || request.cover_thumb_url ? t("admin.proposedCover") : t("admin.coverGone") }}
            </figcaption>
          </figure>
        </div>

        <dl v-if="proposedFields(request, locale.locale).length" class="proposed-fields">
          <template v-for="field in proposedFields(request, locale.locale)" :key="field.key">
            <dt class="fine subtle">{{ field.label }}</dt>
            <dd>
              <span v-if="field.before !== null" class="field-before" :class="{ 'was-empty': field.wasEmpty, same: field.same }">
                <span class="field-tag finer subtle">{{ t("admin.now") }}</span>
                <span class="field-value" :class="{ long: field.key === 'description' }">{{ field.before }}</span>
              </span>
              <span class="field-after" :class="{ cleared: field.cleared }">
                <span v-if="field.before !== null" class="field-tag finer subtle">{{ t("admin.proposed") }}</span>
                <span class="field-value" :class="{ long: field.key === 'description' }">{{ field.value }}</span>
                <span v-if="field.same" class="finer subtle">{{ t("admin.noChange") }}</span>
              </span>
            </dd>
          </template>
        </dl>

        <div v-if="mergeTarget(request) != null" class="merge-target">
          <p class="fine">{{ t("admin.mergeInto") }}</p>
          <RouterLink v-if="extraBooks[mergeTarget(request)!]" :to="bookPath(mergeTarget(request)!)" class="admin-book-link">
            <BookCover :book="extraBooks[mergeTarget(request)!]!" size="sm" />
            <span>
              <strong>{{ extraBooks[mergeTarget(request)!]!.title }}</strong>
              <span v-if="extraBooks[mergeTarget(request)!]!.authors" class="fine subtle">{{ extraBooks[mergeTarget(request)!]!.authors }}</span>
            </span>
          </RouterLink>
          <RouterLink v-else :to="bookPath(mergeTarget(request)!)" class="fine">
            {{ t("admin.bookN", { id: mergeTarget(request) }) }}
          </RouterLink>
        </div>

        <blockquote v-if="request.note" class="fine request-note">{{ request.note }}</blockquote>

        <form v-if="rejecting === request.id" class="stack" @submit.prevent="reject(request)">
          <label class="field">
            <span>{{ t("admin.rejectNote") }}</span>
            <textarea v-model="rejectNote" v-autosize name="reject-note" rows="2" maxlength="2000" :placeholder="t('common.optional')" />
          </label>
          <div class="btn-row">
            <button class="btn btn-danger btn-sm" type="submit" :disabled="busyId === request.id">{{ t("admin.reject") }}</button>
            <button class="btn btn-ghost btn-sm" type="button" @click="rejecting = null">{{ t("common.cancel") }}</button>
          </div>
        </form>
        <div v-else class="btn-row">
          <button class="btn btn-primary btn-sm" type="button" :disabled="busyId === request.id" @click="approve(request)">
            {{ t("admin.approve") }}
          </button>
          <button class="btn btn-ghost btn-sm" type="button" :disabled="busyId === request.id" @click="startReject(request)">
            {{ t("admin.reject") }}
          </button>
          <button
            v-if="request.work_id != null || request.edition_id != null"
            class="btn btn-ghost btn-sm admin-edit"
            type="button"
            :disabled="opening === request.id"
            @click="editAsAdmin(request)"
          >
            {{ t("admin.editAsAdmin") }}
          </button>
        </div>
      </li>
    </ol>
    <EditRequestSheet
      v-if="editing"
      admin
      :book="editing.book"
      :edition="editing.edition"
      @saved="edited"
      @close="editing = null"
    />
  </section>
</template>

<style scoped>
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
}

.admin-item-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}

.admin-book {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: var(--space-4);
}

.admin-book-link {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  color: inherit;
  text-decoration: none;
}

.admin-book-link > span {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.merge-target {
  display: grid;
  gap: var(--space-2);
}

.merge-target p {
  margin: 0;
}

.proposed-cover {
  display: grid;
  justify-items: center;
  gap: var(--space-1);
  margin: 0;
}

.proposed-fields {
  display: grid;
  grid-template-columns: minmax(6rem, max-content) 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
}

.proposed-fields dd {
  display: grid;
  gap: 2px;
  margin: 0;
  min-width: 0;
}

.field-before,
.field-after {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
  min-width: 0;
}

.field-tag {
  flex: 0 0 auto;
  min-width: 4.5rem;
}

.field-value {
  flex: 0 1 auto;
  min-width: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.field-value.long {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 4;
  line-clamp: 4;
  overflow: hidden;
}

.field-before .field-value {
  color: var(--text-muted);
  text-decoration: line-through;
}

.field-before.was-empty .field-value {
  font-style: italic;
  text-decoration: none;
}

.field-before.same .field-value {
  text-decoration: none;
}

.field-after.cleared .field-value {
  font-style: italic;
  color: var(--text-muted);
}

/* On a phone the field's name goes above its values, so each value keeps its tag beside it. */
@media (max-width: 34rem) {
  .proposed-fields {
    grid-template-columns: 1fr;
  }

  .proposed-fields dd + dt {
    margin-top: var(--space-2);
  }
}

.request-note {
  margin: 0;
  padding-left: var(--space-3);
  border-left: 2px solid var(--border-strong);
  white-space: pre-wrap;
}
</style>
