<script setup lang="ts">
import { vAutosize } from "../../autosize";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { BookDetailOut, EditRequestOut } from "../../api/generated";
import { mergeTarget, proposedFields, requestKindLabel } from "../../admin";
import { bookPath } from "../../constants";
import { formatClubDate } from "../../i18n/dates";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";

/**
 * The catalog queue: corrections of books and editions, cover photos that
 * would replace a cover, and "these are the same book". Approving applies the
 * request (an admin's values, which no catalog source overwrites); rejecting
 * closes it with an optional note.
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
 * Books the answer does not carry, by work id: a merge's target, and a
 * request whose `book` is null although its work exists (the server fills
 * `book` for only one of several open requests on the same edition). Null
 * when the work is gone.
 */
const extraBooks = ref<Record<number, BookDetailOut | null>>({});

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

/** Reads the books the queue names but does not carry, one request each. */
async function loadExtraBooks() {
  const ids = new Set<number>();
  for (const request of items.value) {
    if (!request.book && request.work_id != null) ids.add(request.work_id);
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

/** The book a request is about: the answer's, else the one read by its work id. */
function bookOf(request: EditRequestOut) {
  return request.book ?? (request.work_id != null ? extraBooks.value[request.work_id] ?? null : null);
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
          <RouterLink v-if="bookOf(request)" :to="bookPath(bookOf(request)!)" class="admin-book-link">
            <BookCover :book="bookOf(request)!" size="sm" />
            <span>
              <strong>{{ bookOf(request)!.title }}</strong>
              <span v-if="bookOf(request)!.authors" class="fine subtle">{{ bookOf(request)!.authors }}</span>
              <span v-if="request.edition_id" class="finer subtle">{{ t("admin.edition", { id: request.edition_id }) }}</span>
            </span>
          </RouterLink>
          <RouterLink v-else-if="request.work_id != null && extraBooks[request.work_id] === undefined" :to="bookPath(request.work_id)" class="fine">
            {{ t("admin.bookN", { id: request.work_id }) }}
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
            <dd :class="{ cleared: field.cleared }">{{ field.value }}</dd>
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
        </div>
      </li>
    </ol>
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
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.proposed-fields dd.cleared {
  font-style: italic;
  color: var(--text-muted);
}

.request-note {
  margin: 0;
  padding-left: var(--space-3);
  border-left: 2px solid var(--border-strong);
  white-space: pre-wrap;
}
</style>
