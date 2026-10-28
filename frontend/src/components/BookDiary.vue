<script setup lang="ts">
import { vAutosize } from "../autosize";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import type { Visibility } from "../types";
import type { Status } from "../constants";
import { countEntries, defaultSpoilerUpto } from "../diary";
import { draftKey, loadDraft, saveDraft } from "../drafts";
import { tp } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";

const { t } = useI18n();
import type { DiaryEntry as Entry, PickThreadOut } from "../types";
import DiaryEntry from "./DiaryEntry.vue";
import SpoilerControl from "./SpoilerControl.vue";

/** What the thread shows once loaded: a work's diary or a club pick's discussion. */
type Thread = {
  my_progress: number | null;
  my_status: Status | null;
  items: Entry[];
  /** False on an ended pick: its discussion is read-only. */
  can_post: boolean;
};

const props = defineProps<{
  /** The work's numeric id: its diary entries, plus the club posts of the viewer's clubs. */
  workId?: number;
  /**
   * A club pick's discussion instead (`pickId` null: the current pick).
   * `currentPickId` is the club's current pick as `ClubOut.current_pick` has
   * it: null means there is none, so the current pick's discussion is not
   * asked for (it would answer 404 `No club pick yet`); left out, it is asked.
   */
  club?: { slug: string; pickId?: number | null; currentPickId?: number | null };
  /** Show only the newest N entries until the reader asks for the rest. */
  preview?: number;
  heading?: string;
}>();

const emit = defineEmits<{
  /** A new entry or reply was posted (the club page refreshes its feed). */
  posted: [entry: Entry];
}>();

const REPLY_PREVIEW = 2;

const toast = useToast();
const session = useSession();
const route = useRoute();
const clubs = useClubs();
const diary = ref<Thread | null>(null);
/** No clubs: no meeting to prepare for, so the prompts leave it out. */
const solo = computed(() => !props.club && !session.user?.clubs.length);
/** The club has no current pick, so there is no discussion to show. */
const noPick = ref(false);
const loaded = ref(false);
const error = ref("");
const showAll = ref(false);
const busyId = ref<number | null>(null);

const draft = ref("");
const spoiler = ref<number | null>(null);
/** Diary entries can be kept to oneself; club posts and replies cannot. */
const onlyMe = ref(false);
const spoilerTouched = ref(false);
const posting = ref(false);
const composer = ref<HTMLTextAreaElement | null>(null);
const composeForm = ref<HTMLFormElement | null>(null);
// The composer rests as one line and opens up once someone starts writing.
const composing = ref(false);
const composeOpen = computed(() => composing.value || draft.value.length > 0);

const replyTo = ref<number | null>(null);
const replyDraft = ref("");
const replySpoiler = ref<number | null>(null);
const expandedReplies = ref<Set<number>>(new Set());

const threadKey = computed(() =>
  props.club ? `club:${props.club.slug}:${props.club.pickId ?? "current"}` : String(props.workId),
);
const draftId = computed(() => draftKey(session.user?.username, threadKey.value));
const replyDraftId = (entryId: number) => draftKey(session.user?.username, threadKey.value, "reply", entryId);
watch(draft, (text) => saveDraft(draftId.value, text));
watch(replyDraft, (text) => {
  if (replyTo.value != null) saveDraft(replyDraftId(replyTo.value), text);
});

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

function fromPickThread(thread: PickThreadOut): Thread {
  return {
    my_progress: thread.my_progress,
    my_status: thread.my_status,
    items: thread.items,
    can_post: thread.can_post,
  };
}

async function fetchThread(): Promise<Thread> {
  if (props.club) return fromPickThread(await api.pickThread(props.club.slug, props.club.pickId));
  return { ...(await api.diary(props.workId!)), can_post: true };
}

/** The club answer already says there is no current pick: nothing to ask for. */
const knownNoPick = computed(
  () => Boolean(props.club && !props.club.pickId && props.club.currentPickId === null),
);

async function load() {
  if (knownNoPick.value) {
    noPick.value = true;
    diary.value = null;
    error.value = "";
    loaded.value = true;
    return;
  }
  try {
    diary.value = await fetchThread();
    noPick.value = false;
    error.value = "";
    resetSpoilerDefault();
    await jumpToAnchor();
  } catch (err) {
    if (props.club && isClubGone(err)) clubs.markGone(props.club.slug);
    if (props.club && err instanceof ApiError && err.status === 404 && err.detail === "No club pick yet") {
      noPick.value = true;
      diary.value = null;
      error.value = "";
    } else {
      error.value = err instanceof ApiError ? err.message : t("diary.loadFailed");
    }
  } finally {
    loaded.value = true;
  }
}

function post(body: { body: string; spoiler_upto: number | null; parent_id?: number }) {
  if (props.club) return api.addPickPost(props.club.slug, props.club.pickId, body);
  return api.addDiaryEntry(props.workId!, {
    ...body,
    ...(body.parent_id == null && onlyMe.value ? { visibility: "only_me" as Visibility } : {}),
  });
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
  const key = draftId.value;
  try {
    const created = await post({ body, spoiler_upto: spoiler.value });
    // Posted: forget the draft now, even if the page changes before the field clears.
    saveDraft(key, "");
    diary.value.items = [...diary.value.items, created];
    emit("posted", created);
    draft.value = "";
    composing.value = false;
    onlyMe.value = false;
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
  replyDraft.value = loadDraft(replyDraftId(entry.id));
  replySpoiler.value = defaultSpoilerUpto(myProgress.value, myStatus.value);
}

async function submitReply(parent: Entry) {
  const body = replyDraft.value.trim();
  if (!body || posting.value) return;
  posting.value = true;
  try {
    const created = await post({ body, spoiler_upto: replySpoiler.value, parent_id: parent.id });
    parent.replies = [...parent.replies, created];
    emit("posted", created);
    expandReplies(parent.id);
    saveDraft(replyDraftId(parent.id), "");
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

async function edit(entry: Entry, body: string, spoilerUpto: number | null, visibility?: Visibility) {
  busyId.value = entry.id;
  try {
    replaceEntry(
      await api.editDiaryEntry(entry.id, {
        body,
        spoiler_upto: spoilerUpto,
        ...(visibility ? { visibility } : {}),
      }),
    );
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

// A tap elsewhere folds an empty composer away again. Listening for the tap,
// not for blur, keeps its own buttons usable on phones that never focus them.
function foldComposer(event: PointerEvent) {
  if (!draft.value.trim() && !composeForm.value?.contains(event.target as Node)) composing.value = false;
}

async function focusComposer() {
  showAll.value = true;
  composing.value = true;
  await nextTick();
  composer.value?.scrollIntoView({ behavior: "smooth", block: "center" });
  composer.value?.focus();
}

defineExpose({ focusComposer, reload: load });

onMounted(() => {
  draft.value = loadDraft(draftId.value);
  document.addEventListener("pointerdown", foldComposer);
  void load();
});
onBeforeUnmount(() => document.removeEventListener("pointerdown", foldComposer));
watch(
  // The current pick's discussion also follows the club answer: a pick set
  // since (or the layout's fresh answer) brings its discussion.
  [threadKey, () => (props.club && !props.club.pickId ? props.club.currentPickId : undefined)],
  ([key], [before]) => {
    loaded.value = false;
    if (key !== before) {
      showAll.value = false;
      spoilerTouched.value = false;
      replyTo.value = null;
      draft.value = loadDraft(draftId.value);
    }
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
    <p v-else-if="noPick" class="fine subtle">{{ t("diary.noPick") }}</p>

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
            :show-club="!club"
            :busy="busyId === entry.id"
            @react="react(entry, $event)"
            :no-reply="diary?.can_post === false"
            @reply="startReply(entry)"
            @edit="(body, upto, visibility) => edit(entry, body, upto, visibility)"
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
                v-if="replyTo === entry.id && diary?.can_post"
                class="reply-form"
                @submit.prevent="submitReply(entry)"
              >
                <label class="field">
                  <span class="visually-hidden">{{ t("diary.replyToAria", { name: entry.author }) }}</span>
                  <textarea
                    v-autosize
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
      <div v-else class="diary-empty">
        <p class="fine subtle">{{ club ? t("diary.emptyClub") : solo ? t("diary.emptySolo") : t("diary.empty") }}</p>
        <button v-if="diary?.can_post" class="btn btn-ghost btn-sm" type="button" @click="focusComposer">
          {{ t("diary.writeFirst") }}
        </button>
      </div>
      <p v-if="diary && !diary.can_post" class="fine subtle">{{ t("diary.closed") }}</p>
    </template>

    <form
      v-if="!noPick && (diary?.can_post ?? true)"
      ref="composeForm"
      class="compose"
      :class="{ open: composeOpen }"
      @submit.prevent="submit"
      @focusin="composing = true"
    >
      <label class="field">
        <span class="visually-hidden">{{ club ? t("discussions.writeAria") : t("diary.writeAria") }}</span>
        <textarea
          v-autosize
          ref="composer"
          v-model="draft"
          :rows="composeOpen ? 3 : 1"
          maxlength="1000"
          :placeholder="solo ? t('diary.placeholderSolo') : t('diary.placeholder')"
        />
      </label>
      <div v-show="composeOpen" class="compose-row">
        <SpoilerControl
          :model-value="spoiler"
          @update:model-value="
            (value) => {
              spoiler = value;
              spoilerTouched = true;
            }
          "
        />
        <label v-if="!club" class="only-me fine">
          <input v-model="onlyMe" type="checkbox" />
          {{ t("diary.onlyMe") }}
        </label>
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
/* A flex column, not a grid: a sticky grid item cannot leave its own track. */
.diary {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.diary-empty {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.skeleton-block {
  height: 90px;
}

.show-all {
  align-self: flex-start;
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

.only-me {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: var(--tap);
  margin-right: auto;
}

/* Resting state: one line, however long the placeholder is. */
.compose:not(.open) textarea {
  field-sizing: fixed;
  min-height: var(--tap);
  height: var(--tap);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  resize: none;
}

/* On phones the composer rides above the tab bar while the diary is on screen,
   so writing never means scrolling past every entry first. */
@media (max-width: 1023px) {
  /* A long draft scrolls inside the field instead of covering the diary. */
  .compose.open textarea {
    max-height: 30vh;
  }

  .compose {
    position: sticky;
    bottom: calc(var(--nav-h) + env(safe-area-inset-bottom));
    z-index: 10;
    margin: 0 calc(-1 * var(--space-4));
    padding: var(--space-2) var(--space-4);
    background: var(--bg-translucent-strong);
    backdrop-filter: blur(12px);
    border-top: 1px solid var(--border);
  }
}

@media (min-width: 1024px) {
  .diary {
    max-width: 720px;
  }
}
</style>
