<script setup lang="ts">
import { EllipsisHorizontalIcon } from "@heroicons/vue/24/outline";
import { vAutosize } from "../autosize";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { Status } from "../constants";
import { REACTIONS, guardLine, positionMarker, spoilerLabel } from "../diary";
import { ancestry, focusLost, focusTemporarily, giveFocusBack } from "../focusBack";
import { spoilerGuard } from "../spoilers";
import { isClubAdmin } from "../stores/clubs";
import { useSession } from "../stores/session";
import { postTimeZone } from "../timezone";
import type { DiaryEntry, Visibility } from "../types";
import Avatar from "./Avatar.vue";
import PostActionsSheet from "./PostActionsSheet.vue";
import RelativeTime from "./RelativeTime.vue";
import SpoilerControl from "./SpoilerControl.vue";

/**
 * One post of a diary or a club discussion, as a light row. Its actions sit
 * in one line under the text: the reactions on the left, "Reply" on the
 * right, and editing, deleting (and a club admin's moderation) behind "…".
 *
 * A post the reader is not far enough for waits for a tap (`spoilers.ts`):
 * one the writer flagged shows blurred under "Spoiler bis 70 % · du bist bei
 * 35 %"; one written further on than the reader is folds into one quiet line,
 * "jonas · bei 35 % · du bist bei 0 %". A tap shows it for this view.
 */
const { t } = useI18n();
const session = useSession();

const props = defineProps<{
  entry: DiaryEntry;
  myProgress: number | null;
  myStatus: Status | null;
  /** Replies render smaller and cannot be replied to. */
  compact?: boolean;
  /** No reply button (an ended pick's discussion). */
  noReply?: boolean;
  /** Name the club of a club post (in a work's diary, where both scopes meet). */
  showClub?: boolean;
  busy?: boolean;
}>();

const emit = defineEmits<{
  react: [emoji: string];
  reply: [];
  edit: [body: string, spoilerUpto: number | null, visibility?: Visibility];
  delete: [];
}>();

const root = ref<HTMLElement | null>(null);
const editField = ref<HTMLTextAreaElement | null>(null);
const moreButton = ref<HTMLButtonElement | null>(null);
const revealed = ref(false);
const pickerOpen = ref(false);
const actionsOpen = ref(false);
const editing = ref(false);
const editDraft = ref("");
const editSpoiler = ref<number | null>(null);
const editOnlyMe = ref(false);

const guard = computed(() =>
  revealed.value ? ({ kind: "open" } as const) : spoilerGuard(props.entry, props.myProgress, props.myStatus),
);
/** Written further on than the reader is: the whole post is one quiet line. */
const folded = computed(() => guard.value.kind === "ahead");
/** Flagged by the writer: the text shows blurred under the flag. */
const shielded = computed(() => guard.value.kind === "marked");
const author = computed(() => props.entry.author ?? t("diary.deletedAccount"));
const marker = computed(() => positionMarker(props.entry));
const zone = computed(() => postTimeZone(props.entry, session.user));
/** Only a diary entry (not a reply, not a club post) can be kept to its author. */
const isDiaryRoot = computed(() => props.entry.club == null && props.entry.parent_id == null);
/** Admins and the owner of a post's own club may delete it (moderation). */
const canDelete = computed(() => {
  if (props.entry.mine) return true;
  const club = props.entry.club;
  if (!club) return false;
  return isClubAdmin(session.user?.clubs.find((row) => row.slug === club.slug)?.role);
});
const hasActions = computed(() => props.entry.mine || canDelete.value);
const pickable = computed(() =>
  REACTIONS.filter((emoji) => !props.entry.reactions.some((row) => row.emoji === emoji && row.mine)),
);

/** Shown for this view; the focus moves from the gone button to the post itself. */
async function reveal() {
  revealed.value = true;
  await nextTick();
  if (root.value) focusTemporarily(root.value);
}

function startEdit() {
  editDraft.value = props.entry.body;
  editSpoiler.value = props.entry.spoiler_upto;
  editOnlyMe.value = props.entry.visibility === "only_me";
  editing.value = true;
}

/** The form goes and its button with it: the focus goes back to "…", which opened it. */
async function endEdit() {
  editing.value = false;
  await nextTick();
  if (!focusLost()) return;
  if (moreButton.value?.isConnected) moreButton.value.focus();
  else if (root.value) focusTemporarily(root.value);
}

function saveEdit() {
  const body = editDraft.value.trim();
  if (!body) return;
  emit(
    "edit",
    body,
    editSpoiler.value,
    isDiaryRoot.value ? (editOnlyMe.value ? "only_me" : "audience") : undefined,
  );
  void endEdit();
}

async function act(action: "edit" | "delete") {
  actionsOpen.value = false;
  if (action === "delete") {
    emit("delete");
    return;
  }
  startEdit();
  // After the sheet gave the focus back to "…", which the form replaces.
  await nextTick();
  editField.value?.focus();
}

// The reaction just tapped pops once when its chip appears or changes;
// reactions already there on load stay still.
const popped = ref<string | null>(null);

function react(emoji: string) {
  pickerOpen.value = false;
  popped.value = emoji;
  emit("react", emoji);
}

watch(
  () => props.entry.id,
  () => {
    revealed.value = false;
    editing.value = false;
    actionsOpen.value = false;
  },
);

// A deleted post with replies stays as a tombstone without its "…": the
// focus stays on the post instead of falling to the page.
watch(
  () => props.entry.deleted,
  async (deleted) => {
    if (!deleted) return;
    await nextTick();
    if (focusLost() && root.value) focusTemporarily(root.value);
  },
);

// A deleted post without replies goes: the focus goes to the list it was in.
onBeforeUnmount(() => {
  const el = root.value;
  if (!el || !el.contains(document.activeElement)) return;
  const chain = ancestry(el.parentElement);
  void nextTick(() => {
    if (focusLost()) giveFocusBack(chain, { nearest: true });
  });
});
</script>

<template>
  <article
    :id="`entry-${entry.id}`"
    ref="root"
    class="entry"
    :class="{ compact, deleted: entry.deleted, mine: entry.mine, folded }"
  >
    <button
      v-if="folded"
      class="entry-folded"
      type="button"
      aria-expanded="false"
      @click="reveal"
    >
      <Avatar :username="entry.author ?? ''" :src="entry.author_avatar_url" size="xs" />
      <span class="folded-text">
        <span class="folded-line">{{ guardLine(guard, entry, author) }}</span>
        <span class="folded-hint">{{ t("spoilers.tapToRead") }}</span>
      </span>
    </button>

    <template v-else>
      <header class="entry-meta">
        <Avatar
          :username="entry.author ?? ''"
          :src="entry.author_avatar_url"
          :size="compact ? 'xs' : 'sm'"
        />
        <strong class="entry-author">{{ author }}</strong>
        <span v-if="marker" class="entry-marker" :class="entry.status_at ?? ''">{{ marker }}</span>
        <RelativeTime
          class="finer subtle entry-time"
          :iso="entry.created_at"
          :zone="zone"
          :fallback="entry.created_label"
        />
        <span v-if="entry.edited" class="finer subtle">{{ t("diary.edited") }}</span>
        <RouterLink
          v-if="showClub && entry.club"
          class="badge entry-club"
          :to="`/c/${entry.club.slug}/discussions`"
          :title="t('diary.clubPostIn', { club: entry.club.name })"
        >
          {{ entry.club.name }}
        </RouterLink>
        <span v-if="entry.visibility === 'only_me'" class="badge only-me-badge" :title="t('diary.onlyMeHint')">
          {{ t("diary.onlyMeBadge") }}
        </span>
        <span
          v-if="!entry.deleted && entry.spoiler_upto != null && !shielded"
          class="finer subtle"
        >
          {{ spoilerLabel(entry.spoiler_upto) }}
        </span>
      </header>

      <p v-if="entry.deleted" class="entry-deleted fine subtle">{{ t("diary.deleted") }}</p>

      <form v-else-if="editing" class="entry-edit" @submit.prevent="saveEdit">
        <label class="field">
          <span class="visually-hidden">{{ t("diary.editAria") }}</span>
          <textarea ref="editField" v-autosize v-model="editDraft" rows="3" maxlength="1000" />
        </label>
        <div class="entry-edit-row">
          <SpoilerControl v-model="editSpoiler" />
          <label v-if="isDiaryRoot" class="entry-only-me fine">
            <input v-model="editOnlyMe" type="checkbox" />
            {{ t("diary.onlyMe") }}
          </label>
          <span class="entry-edit-actions">
            <button class="btn btn-ghost btn-sm" type="button" @click="endEdit">
              {{ t("common.cancel") }}
            </button>
            <button
              class="btn btn-primary btn-sm"
              type="submit"
              :disabled="busy || !editDraft.trim()"
            >
              {{ t("common.save") }}
            </button>
          </span>
        </div>
      </form>

      <div v-else class="entry-body" :class="{ shielded }">
        <p class="entry-text" :aria-hidden="shielded ? 'true' : undefined">{{ entry.body }}</p>
        <button v-if="shielded" class="shield-btn" type="button" @click="reveal">
          <span class="fine shield-label">{{ guardLine(guard, entry) }}</span>
          <span class="finer shield-hint">{{ t("spoilers.tapToRead") }}</span>
        </button>
      </div>

      <!-- One line: the reactions on the left, "Reply" and "…" on the right. -->
      <footer v-if="!entry.deleted && !editing && !shielded" class="entry-actions">
        <span class="reaction-row">
          <button
            v-for="row in entry.reactions"
            :key="row.emoji"
            class="chip reaction"
            :class="{ active: row.mine, pop: popped === row.emoji }"
            @animationend="popped = null"
            type="button"
            :aria-pressed="row.mine"
            :title="row.users.join(', ')"
            :disabled="busy"
            @click="react(row.emoji)"
          >
            {{ row.emoji }} <span class="nums">{{ row.count }}</span>
          </button>
          <button
            class="chip reaction add"
            type="button"
            :aria-expanded="pickerOpen"
            :aria-label="t('diary.addReaction')"
            :disabled="busy"
            @click="pickerOpen = !pickerOpen"
          >
            +
          </button>
        </span>
        <!-- The open picker takes a line of its own under the row (CSS order), so
             "Reply" and "…" never sit over it; in the DOM it follows "+". -->
        <span v-if="pickerOpen" class="reaction-picker-line">
          <span class="reaction-picker" role="group" :aria-label="t('diary.reactions')">
            <button
              v-for="emoji in pickable"
              :key="emoji"
              class="reaction-pick"
              type="button"
              :aria-label="t('diary.react', { emoji })"
              @click="react(emoji)"
            >
              {{ emoji }}
            </button>
          </span>
        </span>
        <span class="entry-links">
          <button v-if="!compact && !noReply" class="text-btn entry-reply" type="button" @click="emit('reply')">
            {{ t("common.reply") }}
          </button>
          <button
            v-if="hasActions"
            ref="moreButton"
            class="icon-btn entry-more"
            type="button"
            aria-haspopup="dialog"
            :aria-expanded="actionsOpen"
            :aria-label="t('diary.postActions')"
            @click="actionsOpen = true"
          >
            <EllipsisHorizontalIcon class="entry-more-icon" aria-hidden="true" />
          </button>
        </span>
      </footer>
    </template>

    <PostActionsSheet
      v-if="actionsOpen"
      :title="entry.mine ? t('diary.yourPost') : t('diary.postBy', { name: author })"
      :can-edit="entry.mine"
      :can-delete="canDelete"
      :moderating="!entry.mine"
      @close="actionsOpen = false"
      @act="act"
    />

    <slot />
  </article>
</template>

<style scoped>
/* A light row, not a card: the list around it draws the hairlines
   (BookDiary), and a reply sits indented behind the thread's rule. */
.entry {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-3) 0;
  scroll-margin-top: calc(var(--header-h) + var(--space-4));
}

.entry.compact {
  gap: var(--space-1);
  padding: var(--space-2) 0;
}

.entry-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-2);
}

.entry-meta :deep(.avatar) {
  background: var(--accent-soft);
  color: var(--accent-hover);
}

.entry-marker {
  font-size: var(--text-xs);
  padding: 1px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  color: var(--text-muted);
  white-space: nowrap;
}

.entry-marker.finished {
  background: var(--status-finished-bg);
  color: var(--status-finished-fg);
}

.entry-marker.currently_reading {
  background: var(--status-reading-bg);
  color: var(--status-reading-fg);
}

.entry-marker.did_not_finish {
  background: var(--status-dnf-bg);
  color: var(--status-dnf-fg);
}

.entry-time {
  margin-left: auto;
}

.entry-body {
  position: relative;
}

.entry-text {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--text-muted);
}

.compact .entry-text {
  font-size: var(--text-sm);
}

.entry-body.shielded .entry-text {
  filter: blur(7px);
  user-select: none;
  pointer-events: none;
  min-height: 2.6em;
  opacity: 0.7;
}

.shield-btn {
  position: absolute;
  inset: 0;
  display: grid;
  place-content: center;
  gap: 2px;
  text-align: center;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
}

/* The flag says what it is in words; no alarm colour over the blur. */
.shield-label {
  font-weight: 600;
}

.shield-hint {
  color: var(--text-muted);
}

/* Written further on than the reader is: the whole post is one quiet line,
   a 44px target, and nothing of its text is in the page. */
.entry.folded {
  padding-block: var(--space-1);
}

.entry-folded {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  min-height: var(--tap);
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--text-sm);
  text-align: left;
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-out);
}

.entry-folded:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (prefers-reduced-motion: reduce) {
  .entry-folded:active {
    transform: none;
  }
}

.entry-folded :deep(.avatar) {
  flex: none;
  opacity: 0.8;
}

.folded-text {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 var(--space-2);
  min-width: 0;
}

.folded-hint {
  color: var(--accent-hover);
  font-weight: 600;
  white-space: nowrap;
}

@media (hover: hover) {
  .entry-folded:hover .folded-line {
    color: var(--text);
  }
}

.entry-deleted {
  font-style: italic;
}

.entry-edit {
  display: grid;
  gap: var(--space-2);
}

.entry-edit .field {
  margin-bottom: 0;
}

.entry-edit-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.entry-edit-actions {
  display: flex;
  gap: var(--space-2);
}

.entry-only-me {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: var(--tap);
}

.only-me-badge {
  font-size: var(--text-2xs);
}

.entry-club {
  text-decoration: none;
}

/* One line under the text: the reactions take what room there is (and wrap
   among themselves); "Reply" and "…" keep their place at the right end. The
   open picker wraps onto a line of its own below. */
.entry-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: var(--space-2);
}

.reaction-row {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1);
}

/* 40px pills; the chip's ::before reaches 44px (components.css). */
.chip.reaction {
  min-height: 40px;
  padding: 0 var(--space-3);
  gap: var(--space-1);
  font-size: var(--text-sm);
}

.chip.reaction.add {
  min-width: 40px;
  justify-content: center;
  color: var(--text-muted);
}

.chip.reaction.pop {
  animation: reaction-pop var(--dur-slow) var(--ease-out);
}

@keyframes reaction-pop {
  40% {
    transform: scale(1.18);
  }
}

@media (prefers-reduced-motion: reduce) {
  .chip.reaction.pop {
    animation: none;
  }
}

.reaction-picker-line {
  order: 1;
  flex: 0 0 100%;
  min-width: 0;
}

/* Six 40px buttons are 256px; on a narrower column they wrap inside the pill. */
.reaction-picker {
  display: inline-flex;
  flex-wrap: wrap;
  max-width: 100%;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--surface);
}

.reaction-pick {
  border: 0;
  background: transparent;
  min-width: 40px;
  min-height: 40px;
  font-size: var(--text-lg);
  border-radius: var(--radius-pill);
  transition: transform var(--dur-fast) var(--ease-out);
}

.reaction-pick:active {
  transform: scale(1.25);
  transition-duration: var(--dur-press);
}

@media (prefers-reduced-motion: no-preference) {
  .reaction-picker {
    animation: picker-in var(--dur-base) var(--ease-out);
    transform-origin: left center;
  }
}

@keyframes picker-in {
  from {
    opacity: 0;
    transform: scale(0.9);
  }
}

@media (hover: hover) {
  .reaction-pick:hover {
    background: var(--surface-2);
  }
}

/* Reply is the row's one accent action; "…" is a quiet icon beside it. The
   padding and the icon's target only widen the hit area: the last one lines
   up with the row's right edge. */
.entry-links {
  flex: none;
  display: flex;
  align-items: center;
  margin-inline-end: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
}

.entry-reply {
  min-height: var(--tap);
}

.entry-more-icon {
  width: 20px;
  height: 20px;
}
</style>
