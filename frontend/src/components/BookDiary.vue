<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import { workId } from "../constants";
import type { Status } from "../constants";
import { countEntries, defaultSpoilerUpto } from "../diary";
import { tp } from "../i18n";
import { useToast } from "../stores/toast";

const { t } = useI18n();
import type { Diary, DiaryBookIn, DiaryEntry as Entry } from "../types";
import DiaryEntry from "./DiaryEntry.vue";
import SpoilerControl from "./SpoilerControl.vue";

const props = defineProps<{
  /** "/works/OL1W" */
  workKey: string;
  /** Enough to create the book row the first time anyone writes about it. */
  book: DiaryBookIn;
  /** Show only the newest N entries until the reader asks for the rest. */
  preview?: number;
  heading?: string;
}>();

const REPLY_PREVIEW = 2;

const toast = useToast();
const route = useRoute();
const diary = ref<Diary | null>(null);
const loaded = ref(false);
const error = ref("");
const showAll = ref(false);
const busyId = ref<number | null>(null);

const draft = ref("");
const spoiler = ref<number | null>(null);
const spoilerTouched = ref(false);
const posting = ref(false);
const composer = ref<HTMLTextAreaElement | null>(null);

const replyTo = ref<number | null>(null);
const replyDraft = ref("");
const replySpoiler = ref<number | null>(null);
const expandedReplies = ref<Set<number>>(new Set());

const items = computed(() => diary.value?.items ?? []);
const myProgress = computed(() => diary.value?.my_progress ?? null);
const myStatus = computed<Status | null>(() => diary.value?.my_status ?? null);
const total = computed(() => countEntries(items.value));

const hidden = computed(() => {
  if (!props.preview || showAll.value) return 0;
  return Math.max(0, items.value.length - props.preview);
});
const visible = computed(() =>
  hidden.value ? items.value.slice(-props.preview!) : items.value,
);

function visibleReplies(entry: Entry): Entry[] {
  if (expandedReplies.value.has(entry.id)) return entry.replies;
  return entry.replies.slice(0, REPLY_PREVIEW);
}

function hiddenReplies(entry: Entry): number {
  if (expandedReplies.value.has(entry.id)) return 0;
  return Math.max(0, entry.replies.length - REPLY_PREVIEW);
}

function expandReplies(id: number) {
  expandedReplies.value = new Set([...expandedReplies.value, id]);
}

function resetSpoilerDefault() {
  if (spoilerTouched.value) return;
  spoiler.value = defaultSpoilerUpto(myProgress.value, myStatus.value);
}

async function load() {
  try {
    diary.value = await api.diary(workId(props.workKey));
    error.value = "";
    resetSpoilerDefault();
    await jumpToAnchor();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("diary.loadFailed");
  } finally {
    loaded.value = true;
  }
}

/** Feed links land on `#entry-<id>`; the element only exists once we have data. */
async function jumpToAnchor() {
  const match = /^#entry-(\d+)$/.exec(route.hash);
  if (!match || !diary.value) return;
  const id = Number(match[1]);
  const parent = diary.value.items.find(
    (entry) => entry.id === id || entry.replies.some((reply) => reply.id === id),
  );
  if (!parent) return;
  showAll.value = true;
  if (parent.id !== id) expandReplies(parent.id);
  await nextTick();
  document.getElementById(`entry-${id}`)?.scrollIntoView({ block: "center" });
}

function fail(err: unknown, fallback: string) {
  toast.show(err instanceof ApiError ? err.message : fallback);
}

function replaceEntry(updated: Entry) {
  if (!diary.value) return;
  diary.value.items = diary.value.items.map((entry) => {
    if (entry.id === updated.id) return { ...updated, replies: entry.replies };
    if (entry.replies.some((reply) => reply.id === updated.id)) {
      return {
        ...entry,
        replies: entry.replies.map((reply) => (reply.id === updated.id ? updated : reply)),
      };
    }
    return entry;
  });
}

async function submit() {
  const body = draft.value.trim();
  if (!body || posting.value || !diary.value) return;
  posting.value = true;
  try {
    const created = await api.addDiaryEntry(workId(props.workKey), {
      body,
      spoiler_upto: spoiler.value,
      book: diary.value.book_id ? undefined : props.book,
    });
    diary.value.items = [...diary.value.items, created];
    if (!diary.value.book_id) await load();
    draft.value = "";
    spoilerTouched.value = false;
    resetSpoilerDefault();
  } catch (err) {
    fail(err, t("diary.postFailed"));
  } finally {
    posting.value = false;
  }
}

function startReply(entry: Entry) {
  replyTo.value = entry.id;
  replyDraft.value = "";
  replySpoiler.value = defaultSpoilerUpto(myProgress.value, myStatus.value);
}

async function submitReply(parent: Entry) {
  const body = replyDraft.value.trim();
  if (!body || posting.value) return;
  posting.value = true;
  try {
    const created = await api.addDiaryEntry(workId(props.workKey), {
      body,
      spoiler_upto: replySpoiler.value,
      parent_id: parent.id,
    });
    parent.replies = [...parent.replies, created];
    expandReplies(parent.id);
    replyTo.value = null;
    replyDraft.value = "";
  } catch (err) {
    fail(err, t("diary.replyFailed"));
  } finally {
    posting.value = false;
  }
}

async function react(entry: Entry, emoji: string) {
  busyId.value = entry.id;
  try {
    replaceEntry(await api.reactToEntry(entry.id, emoji));
  } catch (err) {
    fail(err, t("diary.reactFailed"));
  } finally {
    busyId.value = null;
  }
}

async function edit(entry: Entry, body: string, spoilerUpto: number | null) {
  busyId.value = entry.id;
  try {
    replaceEntry(await api.editDiaryEntry(entry.id, { body, spoiler_upto: spoilerUpto }));
    toast.show(t("diary.updated"));
  } catch (err) {
    fail(err, t("diary.saveFailed"));
  } finally {
    busyId.value = null;
  }
}

async function remove(entry: Entry) {
  if (!diary.value) return;
  busyId.value = entry.id;
  try {
    await api.deleteDiaryEntry(entry.id);
    if (entry.parent_id == null && entry.replies.length) {
      replaceEntry({ ...entry, body: "", deleted: true, spoiler_upto: null, reactions: [] });
    } else {
      diary.value.items = diary.value.items
        .filter((row) => row.id !== entry.id)
        .map((row) => ({ ...row, replies: row.replies.filter((reply) => reply.id !== entry.id) }));
    }
    toast.show(t("diary.deleted"));
  } catch (err) {
    fail(err, t("diary.deleteFailed"));
  } finally {
    busyId.value = null;
  }
}

async function focusComposer() {
  showAll.value = true;
  await nextTick();
  composer.value?.scrollIntoView({ behavior: "smooth", block: "center" });
  composer.value?.focus();
}

defineExpose({ focusComposer, reload: load });

onMounted(load);
watch(
  () => props.workKey,
  () => {
    loaded.value = false;
    showAll.value = false;
    spoilerTouched.value = false;
    void load();
  },
);
</script>

<template>
  <section class="diary">
    <div class="section-head">
      <h2>{{ heading ?? t("diary.heading") }}</h2>
      <span v-if="total" class="fine subtle nums">
        {{ tp("diary.entries", total) }}
      </span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton skeleton-block" aria-hidden="true" />

    <template v-else>
      <button v-if="hidden" class="text-btn show-all" type="button" @click="showAll = true">
        {{ tp("diary.showEarlier", hidden) }}
      </button>

      <ol v-if="visible.length" class="diary-list">
        <li v-for="entry in visible" :key="entry.id">
          <DiaryEntry
            :entry="entry"
            :my-progress="myProgress"
            :my-status="myStatus"
            :busy="busyId === entry.id"
            @react="react(entry, $event)"
            @reply="startReply(entry)"
            @edit="(body, upto) => edit(entry, body, upto)"
            @delete="remove(entry)"
          >
            <div v-if="entry.replies.length || replyTo === entry.id" class="replies">
              <DiaryEntry
                v-for="reply in visibleReplies(entry)"
                :key="reply.id"
                :entry="reply"
                :my-progress="myProgress"
                :my-status="myStatus"
                compact
                :busy="busyId === reply.id"
                @react="react(reply, $event)"
                @edit="(body, upto) => edit(reply, body, upto)"
                @delete="remove(reply)"
              />
              <button
                v-if="hiddenReplies(entry)"
                class="text-btn"
                type="button"
                @click="expandReplies(entry.id)"
              >
                {{ tp("diary.moreReplies", hiddenReplies(entry)) }}
              </button>

              <form
                v-if="replyTo === entry.id"
                class="reply-form"
                @submit.prevent="submitReply(entry)"
              >
                <label class="field">
                  <span class="visually-hidden">{{ t("diary.replyToAria", { name: entry.author }) }}</span>
                  <textarea
                    v-model="replyDraft"
                    rows="2"
                    maxlength="1000"
                    :placeholder="t('diary.replyPlaceholder', { name: entry.author })"
                    autofocus
                  />
                </label>
                <div class="compose-row">
                  <SpoilerControl v-model="replySpoiler" />
                  <span class="compose-actions">
                    <button class="btn btn-ghost btn-sm" type="button" @click="replyTo = null">
                      {{ t("common.cancel") }}
                    </button>
                    <button
                      class="btn btn-primary btn-sm"
                      type="submit"
                      :disabled="posting || !replyDraft.trim()"
                    >
                      {{ t("common.reply") }}
                    </button>
                  </span>
                </div>
              </form>
            </div>
          </DiaryEntry>
        </li>
      </ol>
      <p v-else class="fine subtle">
        {{ t("diary.empty") }}
      </p>
    </template>

    <form class="compose" @submit.prevent="submit">
      <label class="field">
        <span class="visually-hidden">{{ t("diary.writeAria") }}</span>
        <textarea
          ref="composer"
          v-model="draft"
          rows="3"
          maxlength="1000"
          :placeholder="t('diary.placeholder')"
        />
      </label>
      <div class="compose-row">
        <SpoilerControl
          :model-value="spoiler"
          @update:model-value="
            (value) => {
              spoiler = value;
              spoilerTouched = true;
            }
          "
        />
        <button
          class="btn btn-primary btn-sm"
          type="submit"
          :disabled="posting || !draft.trim() || !loaded"
        >
          {{ t("common.post") }}
        </button>
      </div>
    </form>
  </section>
</template>

<style scoped>
.diary {
  display: grid;
  gap: var(--space-3);
}

.skeleton-block {
  height: 90px;
}

.show-all {
  justify-self: start;
}

.diary-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.replies {
  display: grid;
  gap: var(--space-2);
  margin-top: var(--space-1);
  padding-left: var(--space-3);
  border-left: 2px solid var(--border);
}

.replies .text-btn {
  justify-self: start;
  font-size: var(--text-sm);
}

.compose,
.reply-form {
  display: grid;
  gap: var(--space-2);
}

.compose .field,
.reply-form .field {
  margin-bottom: 0;
}

.compose-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-2);
}

.compose-actions {
  display: flex;
  gap: var(--space-2);
}

@media (min-width: 1024px) {
  .diary {
    max-width: 720px;
  }
}
</style>
