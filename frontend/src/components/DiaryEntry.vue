<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { Status } from "../constants";
import { REACTIONS, isShielded, positionMarker, spoilerLabel } from "../diary";
import { formatClubDate } from "../i18n/dates";
import { useClub } from "../stores/club";
import { useLocale } from "../stores/locale";
import type { DiaryEntry } from "../types";
import Avatar from "./Avatar.vue";
import SpoilerControl from "./SpoilerControl.vue";

const { t } = useI18n();
const club = useClub();
const locale = useLocale();

const props = defineProps<{
  entry: DiaryEntry;
  myProgress: number | null;
  myStatus: Status | null;
  /** Replies render smaller and cannot be replied to. */
  compact?: boolean;
  busy?: boolean;
}>();

const emit = defineEmits<{
  react: [emoji: string];
  reply: [];
  edit: [body: string, spoilerUpto: number | null];
  delete: [];
}>();

const revealed = ref(false);
const pickerOpen = ref(false);
const editing = ref(false);
const editDraft = ref("");
const editSpoiler = ref<number | null>(null);
const confirming = ref(false);
let confirmTimer: ReturnType<typeof setTimeout> | null = null;

const shielded = computed(
  () => !revealed.value && isShielded(props.entry, props.myProgress, props.myStatus),
);
const marker = computed(() => positionMarker(props.entry));
const when = computed(
  () =>
    formatClubDate(props.entry.created_at, club.timezone, locale.locale) ||
    props.entry.created_label,
);
const pickable = computed(() =>
  REACTIONS.filter((emoji) => !props.entry.reactions.some((row) => row.emoji === emoji && row.mine)),
);

function startEdit() {
  editDraft.value = props.entry.body;
  editSpoiler.value = props.entry.spoiler_upto;
  editing.value = true;
}

function saveEdit() {
  const body = editDraft.value.trim();
  if (!body) return;
  emit("edit", body, editSpoiler.value);
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

function react(emoji: string) {
  pickerOpen.value = false;
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
        :username="entry.author"
        :src="entry.author_avatar_url"
        :size="compact ? 'xs' : 'sm'"
      />
      <strong class="entry-author">{{ entry.author }}</strong>
      <span v-if="marker" class="entry-marker" :class="entry.status_at ?? ''">{{ marker }}</span>
      <span class="finer subtle entry-time">{{ when }}</span>
      <span v-if="entry.edited" class="finer subtle">{{ t("diary.edited") }}</span>
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
        <textarea v-model="editDraft" rows="3" maxlength="1000" />
      </label>
      <div class="entry-edit-row">
        <SpoilerControl v-model="editSpoiler" />
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
        <span class="kicker">{{ spoilerLabel(entry.spoiler_upto) }}</span>
        <span class="fine">{{ t("diary.youreAt", { n: myProgress ?? 0 }) }}</span>
      </button>
    </div>

    <footer v-if="!entry.deleted && !editing" class="entry-actions">
      <span class="reaction-row">
        <button
          v-for="row in entry.reactions"
          :key="row.emoji"
          class="chip reaction"
          :class="{ active: row.mine }"
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
        <button v-if="!compact" class="text-btn" type="button" @click="emit('reply')">
          {{ t("common.reply") }}
        </button>
        <template v-if="entry.mine">
          <button class="text-btn" type="button" @click="startEdit">{{ t("common.edit") }}</button>
          <button
            class="text-btn"
            :class="{ danger: confirming }"
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
.entry {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  scroll-margin-top: calc(var(--header-h) + var(--space-4));
}

.entry.compact {
  padding: var(--space-2) var(--space-3);
  background: var(--surface-2);
  border-color: transparent;
}

.entry.deleted {
  background: transparent;
  border-style: dashed;
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

.shield-btn .kicker {
  margin: 0;
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

.chip.reaction {
  min-height: 28px;
  padding: 0 var(--space-2);
  font-size: var(--text-sm);
}

.chip.reaction.add {
  min-width: 28px;
  justify-content: center;
  color: var(--text-muted);
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
  min-width: 32px;
  min-height: 28px;
  font-size: var(--text-md);
  border-radius: var(--radius-pill);
}

.reaction-pick:hover {
  background: var(--surface-2);
}

.entry-links {
  display: flex;
  gap: var(--space-3);
  font-size: var(--text-sm);
}

.text-btn.danger {
  color: var(--danger);
}
</style>
