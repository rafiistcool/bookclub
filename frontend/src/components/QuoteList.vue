<script setup lang="ts">
import { vAutosize } from "../autosize";
import { onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { formatClubDate } from "../i18n/dates";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { QuoteOut, Visibility } from "../types";
import BookCover from "./BookCover.vue";

/**
 * Saved lines. On a book page (`workId`) the viewer's audience's quotes of
 * that work plus a composer; on /quotes (`username`) one reader's quotes
 * across books. Each quote is the audience's or "only me".
 */
const props = defineProps<{
  workId?: number;
  username?: string;
  /** Show each quote's book (the /quotes page). */
  showBook?: boolean;
  heading?: string;
  emptyText?: string;
}>();

const emit = defineEmits<{ changed: [count: number] }>();

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const items = ref<QuoteOut[]>([]);
const loaded = ref(false);
const error = ref("");
const busy = ref(false);

const draft = ref("");
const draftPage = ref<number | "">("");
const draftOnlyMe = ref(false);

const editingId = ref<number | null>(null);
const editBody = ref("");
const editPage = ref<number | "">("");
const editOnlyMe = ref(false);
const confirmingId = ref<number | null>(null);

function fail(err: unknown, fallback: string) {
  toast.show(err instanceof ApiError ? err.message : fallback);
}

function pageOf(value: number | ""): number | null {
  return value === "" || Number.isNaN(Number(value)) ? null : Number(value);
}

function visibilityOf(onlyMe: boolean): Visibility {
  return onlyMe ? "only_me" : "audience";
}

async function load() {
  try {
    items.value = (await api.quotes({ work: props.workId, username: props.username })).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("quotes.loadFailed");
  } finally {
    loaded.value = true;
  }
}

async function add() {
  const body = draft.value.trim();
  if (!body || !props.workId || busy.value) return;
  busy.value = true;
  try {
    const created = await api.addQuote({
      work_id: props.workId,
      body,
      page: pageOf(draftPage.value),
      visibility: visibilityOf(draftOnlyMe.value),
    });
    items.value = [created, ...items.value];
    draft.value = "";
    draftPage.value = "";
    draftOnlyMe.value = false;
    toast.show(t("quotes.saved"));
    emit("changed", items.value.length);
  } catch (err) {
    fail(err, t("quotes.saveFailed"));
  } finally {
    busy.value = false;
  }
}

function startEdit(quote: QuoteOut) {
  editingId.value = quote.id;
  editBody.value = quote.body;
  editPage.value = quote.page ?? "";
  editOnlyMe.value = quote.visibility === "only_me";
}

async function saveEdit(quote: QuoteOut) {
  const body = editBody.value.trim();
  if (!body || busy.value) return;
  busy.value = true;
  try {
    const updated = await api.editQuote(quote.id, {
      body,
      page: pageOf(editPage.value),
      visibility: visibilityOf(editOnlyMe.value),
    });
    items.value = items.value.map((row) => (row.id === updated.id ? updated : row));
    editingId.value = null;
    toast.show(t("quotes.updated"));
  } catch (err) {
    fail(err, t("quotes.saveFailed"));
  } finally {
    busy.value = false;
  }
}

async function remove(quote: QuoteOut) {
  if (confirmingId.value !== quote.id) {
    confirmingId.value = quote.id;
    return;
  }
  confirmingId.value = null;
  busy.value = true;
  try {
    await api.deleteQuote(quote.id);
    items.value = items.value.filter((row) => row.id !== quote.id);
    toast.show(t("quotes.deleted"));
    emit("changed", items.value.length);
  } catch (err) {
    fail(err, t("quotes.deleteFailed"));
  } finally {
    busy.value = false;
  }
}

function when(quote: QuoteOut): string {
  return formatClubDate(quote.created_at, session.user?.timezone || "UTC", locale.locale) || quote.created_label;
}

onMounted(load);
watch(
  () => [props.workId, props.username],
  () => {
    loaded.value = false;
    void load();
  },
);
</script>

<template>
  <section class="quotes">
    <div class="section-head">
      <h2>{{ heading ?? t("quotes.heading") }}</h2>
      <span v-if="items.length" class="fine subtle nums">{{ items.length }}</span>
    </div>

    <form v-if="workId" class="quote-compose" @submit.prevent="add">
      <label class="field">
        <span class="visually-hidden">{{ t("quotes.bodyLabel") }}</span>
        <textarea v-autosize v-model="draft" rows="2" maxlength="1000" :placeholder="t('quotes.placeholder')" />
      </label>
      <div v-if="draft.trim()" class="quote-compose-row">
        <label class="quote-page fine">
          {{ t("quotes.page") }}
          <input v-model.number="draftPage" type="number" min="0" max="20000" inputmode="numeric" />
        </label>
        <label class="quote-only-me fine">
          <input v-model="draftOnlyMe" type="checkbox" />
          {{ t("diary.onlyMe") }}
        </label>
        <button class="btn btn-primary btn-sm" type="submit" :disabled="busy">{{ t("quotes.save") }}</button>
      </div>
    </form>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton quotes-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0" class="fine subtle">{{ emptyText ?? t("quotes.empty") }}</p>
    <ol v-else class="quote-list">
      <li v-for="quote in items" :key="quote.id" class="quote" :class="{ mine: quote.mine }">
        <RouterLink v-if="showBook" class="quote-book" :to="bookPath(quote.book)">
          <BookCover :book="quote.book" size="xs" />
          <span class="fine">{{ quote.book.title }}</span>
        </RouterLink>

        <form v-if="editingId === quote.id" class="quote-edit" @submit.prevent="saveEdit(quote)">
          <label class="field">
            <span class="visually-hidden">{{ t("quotes.bodyLabel") }}</span>
            <textarea v-autosize v-model="editBody" rows="2" maxlength="1000" />
          </label>
          <div class="quote-compose-row">
            <label class="quote-page fine">
              {{ t("quotes.page") }}
              <input v-model.number="editPage" type="number" min="0" max="20000" inputmode="numeric" />
            </label>
            <label class="quote-only-me fine">
              <input v-model="editOnlyMe" type="checkbox" />
              {{ t("diary.onlyMe") }}
            </label>
            <span class="btn-row">
              <button class="btn btn-ghost btn-sm" type="button" @click="editingId = null">{{ t("common.cancel") }}</button>
              <button class="btn btn-primary btn-sm" type="submit" :disabled="busy || !editBody.trim()">{{ t("common.save") }}</button>
            </span>
          </div>
        </form>
        <template v-else>
          <blockquote class="quote-body">{{ quote.body }}</blockquote>
          <p class="finer subtle quote-meta">
            <template v-if="quote.page != null">{{ t("quotes.pageN", { n: quote.page }) }} · </template>
            <template v-if="!quote.mine">
              <RouterLink :to="`/u/${quote.author}`">{{ quote.author }}</RouterLink> ·
            </template>
            {{ when(quote) }}
            <span v-if="quote.visibility === 'only_me'" class="badge only-me-badge" :title="t('quotes.onlyMeHint')">
              {{ t("diary.onlyMeBadge") }}
            </span>
          </p>
          <p v-if="quote.mine" class="quote-actions">
            <button class="text-btn" type="button" @click="startEdit(quote)">{{ t("common.edit") }}</button>
            <button
              class="text-btn"
              :class="{ danger: confirmingId === quote.id }"
              type="button"
              :disabled="busy"
              @click="remove(quote)"
            >
              {{ confirmingId === quote.id ? t("common.deleteConfirm") : t("common.delete") }}
            </button>
          </p>
        </template>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.quotes {
  display: grid;
  gap: var(--space-2);
}

.quotes-skeleton {
  height: 90px;
}

.quote-compose,
.quote-edit {
  display: grid;
  gap: var(--space-2);
}

.quote-compose .field,
.quote-edit .field {
  margin-bottom: 0;
}

.quote-compose-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
}

.quote-page {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}

.quote-page input {
  width: 6rem;
  min-height: 38px;
  padding: 0 var(--space-2);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font: inherit;
}

.quote-only-me {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: var(--tap);
  margin-right: auto;
}

.quote-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.quote {
  display: grid;
  gap: var(--space-1);
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.quote-book {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  color: inherit;
  text-decoration: none;
  font-weight: 600;
}

.quote-body {
  margin: 0;
  padding-left: var(--space-3);
  border-left: 3px solid var(--accent-line);
  font-family: var(--serif);
  font-size: var(--text-md);
  line-height: var(--leading-snug);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.quote-meta a {
  color: inherit;
}

.only-me-badge {
  margin-left: var(--space-1);
  font-size: var(--text-2xs);
}

.quote-actions {
  display: flex;
  gap: var(--space-1);
  margin: 0 calc(-1 * var(--space-2));
}

@media (min-width: 1024px) {
  .quotes {
    max-width: 720px;
  }
}
</style>
