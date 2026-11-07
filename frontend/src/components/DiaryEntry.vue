<script setup lang="ts">
import { vAutosize } from "../autosize";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { Status } from "../constants";
import { REACTIONS, isShielded, positionMarker, spoilerLabel } from "../diary";
import { isClubAdmin } from "../stores/clubs";
import { useSession } from "../stores/session";
import { postTimeZone } from "../timezone";
import type { DiaryEntry, Visibility } from "../types";
import Avatar from "./Avatar.vue";
import RelativeTime from "./RelativeTime.vue";
import SpoilerControl from "./SpoilerControl.vue";

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

const revealed = ref(false);
const pickerOpen = ref(false);
const editing = ref(false);
const editDraft = ref("");
const editSpoiler = ref<number | null>(null);
const editOnlyMe = ref(false);
const confirming = ref(false);
let confirmTimer: ReturnType<typeof setTimeout> | null = null;

const shielded = computed(
  () => !revealed.value && isShielded(props.entry, props.myProgress, props.myStatus),
);
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
const pickable = computed(() =>
  REACTIONS.filter((emoji) => !props.entry.reactions.some((row) => row.emoji === emoji && row.mine)),
);

function startEdit() {
  editDraft.value = props.entry.body;
  editSpoiler.value = props.entry.spoiler_upto;
  editOnlyMe.value = props.entry.visibility === "only_me";
  editing.value = true;
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
  editing.value = false;
}

function askDelete() {
  if (confirming.value) {
    confirming.value = false;
    emit("delete");
    return;
  }
  confirming.value = true;
  if (confirmTimer) clearTimeout(confirmTimer);
  confirmTimer = setTimeout(() => (confirming.value = false), 4000);
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
    confirming.value = false;
  },
);
</script>

<template>
  <article
    :id="`entry-${entry.id}`"
    class="entry"
    :class="{ compact, deleted: entry.deleted, mine: entry.mine }"
  >
    <header class="entry-meta">
      <Avatar
        :username="entry.author ?? ''"
        :src="entry.author_avatar_url"
        :size="compact ? 'xs' : 'sm'"
      />
      <strong class="entry-author">{{ entry.author ?? t("diary.deletedAccount") }}</strong>
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
        {{ spoilerLabel(entry.spoiler_upto).toLowerCase() }}
      </span>
    </header>

    <p v-if="entry.deleted" class="entry-deleted fine subtle">{{ t("diary.deleted") }}</p>

    <form v-else-if="editing" class="entry-edit" @submit.prevent="saveEdit">
      <label class="field">
        <span class="visually-hidden">{{ t("diary.editAria") }}</span>
        <textarea v-autosize v-model="editDraft" rows="3" maxlength="1000" />
      </label>
      <div class="entry-edit-row">
        <SpoilerControl v-model="editSpoiler" />
        <label v-if="isDiaryRoot" class="entry-only-me fine">
          <input v-model="editOnlyMe" type="checkbox" />
          {{ t("diary.onlyMe") }}
        </label>
        <span class="entry-edit-actions">
          <button class="btn btn-ghost btn-sm" type="button" @click="editing = false">
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
      <p class="entry-text">{{ entry.body }}</p>
      <button
        v-if="shielded"
        class="shield-btn"
        type="button"
        :aria-label="t('diary.revealAria', { label: spoilerLabel(entry.spoiler_upto) })"
        @click="revealed = true"
      >
        <span class="fine shield-label">{{ spoilerLabel(entry.spoiler_upto) }}</span>
        <span class="fine">{{ t("diary.youreAt", { n: myProgress ?? 0 }) }}</span>
      </button>
    </div>

    <footer v-if="!entry.deleted && !editing" class="entry-actions">
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
        <span v-if="pickerOpen" class="reaction-picker" role="group" :aria-label="t('diary.reactions')">
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
        <button v-if="!compact && !noReply" class="text-btn" type="button" @click="emit('reply')">
          {{ t("common.reply") }}
        </button>
        <button v-if="entry.mine" class="text-btn quiet" type="button" @click="startEdit">{{ t("common.edit") }}</button>
        <template v-if="canDelete">
          <button
            class="text-btn"
            :class="confirming ? 'danger' : 'quiet'"
            type="button"
            :disabled="busy"
            @click="askDelete"
          >
            {{ confirming ? t("common.deleteConfirm") : t("common.delete") }}
          </button>
        </template>
      </span>
    </footer>

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

.shield-label {
  font-weight: 600;
  color: var(--accent);
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

.entry-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.reaction-row {
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

.reaction-picker {
  display: inline-flex;
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

/* The padding only widens the targets: the outer texts line up with the
   row's edges, wherever the links wrap to. */
.entry-links {
  display: flex;
  gap: var(--space-1);
  margin-inline: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
}

/* Reply is the row's one accent action; edit and delete stay quiet until
   delete asks to be confirmed. */
.text-btn.quiet {
  color: var(--text-muted);
}

.text-btn.danger {
  color: var(--danger);
}
</style>
