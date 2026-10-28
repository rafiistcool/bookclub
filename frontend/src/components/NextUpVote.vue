<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { formatClubDate } from "../i18n/dates";
import { tp } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useToast } from "../stores/toast";
import type { NextUpVote as NextUpVoteState, VoteNomination } from "../types";
import BookCover from "./BookCover.vue";
import BottomSheet from "./BottomSheet.vue";
import MeetingSheet from "./MeetingSheet.vue";

const props = defineProps<{
  slug: string;
  /** Admins and the owner confirm the winner and set the deadline. */
  canManage: boolean;
}>();

const emit = defineEmits<{ applied: [] }>();

const { t } = useI18n();
const locale = useLocale();
const clubs = useClubs();
const toast = useToast();

const vote = ref<NextUpVoteState | null>(null);
const error = ref("");
const loaded = ref(false);
const pendingId = ref<number | null>(null);
const confirming = ref<VoteNomination | null>(null);
const editingDeadline = ref(false);
const deadlineDraft = ref("");

const nominations = computed(() =>
  [...(vote.value?.nominations ?? [])].sort(
    (a, b) => b.votes - a.votes || a.book.title.localeCompare(b.book.title),
  ),
);

const totalVotes = computed(() =>
  nominations.value.reduce((sum, row) => sum + row.votes, 0),
);

const closes = computed(() =>
  vote.value?.closes_at
    ? formatClubDate(vote.value.closes_at, vote.value.timezone, locale.locale) || vote.value.closes_label
    : "",
);

function fail(err: unknown, fallback: string) {
  if (isClubGone(err)) clubs.markGone(props.slug);
  toast.show(err instanceof ApiError ? err.message : fallback);
}

async function load() {
  try {
    vote.value = await api.vote(props.slug);
    error.value = "";
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(props.slug);
    error.value = err instanceof ApiError ? err.message : t("vote.loadFailed");
  } finally {
    loaded.value = true;
  }
}

async function cast(row: VoteNomination) {
  pendingId.value = row.id;
  try {
    vote.value = await api.castVote(props.slug, row.id);
  } catch (err) {
    fail(err, t("vote.saveFailed"));
  } finally {
    pendingId.value = null;
  }
}

async function confirm(meetingAt: string | null) {
  const row = confirming.value;
  confirming.value = null;
  if (!row) return;
  pendingId.value = row.id;
  try {
    const result = await api.applyVote(props.slug, row.id, meetingAt);
    vote.value = result.vote;
    clubs.syncPick(props.slug, result.pick);
    toast.show(t("vote.confirmed", { title: row.book.title }));
    emit("applied");
  } catch (err) {
    fail(err, t("vote.confirmFailed"));
  } finally {
    pendingId.value = null;
  }
}

function openDeadline() {
  deadlineDraft.value = vote.value?.closes_local ?? "";
  editingDeadline.value = true;
}

async function saveDeadline(value: string | null) {
  editingDeadline.value = false;
  try {
    vote.value = await api.setVoteDeadline(props.slug, value);
    toast.show(value ? t("vote.deadlineSaved") : t("vote.deadlineCleared"));
  } catch (err) {
    fail(err, t("vote.deadlineFailed"));
  }
}

onMounted(load);
defineExpose({ load });
</script>

<template>
  <section aria-labelledby="next-up-heading">
    <div class="section-head">
      <h2 id="next-up-heading">{{ t("vote.title") }}</h2>
      <span v-if="vote" class="fine subtle nums">
        {{ t("vote.nominationsOf", { current: nominations.length, limit: vote.nomination_limit }) }}
      </span>
    </div>
    <p class="fine muted vote-blurb">
      {{ canManage ? t("vote.blurbAdmin") : t("vote.blurb") }}
    </p>

    <div v-if="vote && vote.vote_id" class="vote-status fine">
      <span class="nums">{{ t("vote.votedOf", { voted: vote.voted_count, total: vote.member_count }) }}</span>
      <span v-if="vote.not_voted.length" class="subtle">
        · {{ t("vote.waitingFor", { names: vote.not_voted.join(", ") }) }}
      </span>
      <span v-if="closes"> · {{ t("vote.closesAt", { when: closes }) }}</span>
      <button v-if="canManage" class="text-btn" type="button" @click="openDeadline">
        {{ vote.closes_at ? t("vote.changeDeadline") : t("vote.setDeadline") }}
      </button>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton skeleton-block" aria-hidden="true" />

    <ol v-else-if="nominations.length" class="vote-list">
      <li
        v-for="(row, index) in nominations"
        :key="row.id"
        class="vote-row"
        :class="{ leading: index === 0 && row.votes > 0 }"
      >
        <RouterLink class="vote-book" :to="bookPath(row.book)">
          <BookCover :book="row.book" size="sm" />
          <span class="vote-meta">
            <strong>{{ row.book.title }}</strong>
            <span v-if="row.book.authors" class="finer subtle">
              {{ row.book.authors }}
            </span>
            <span class="finer subtle">{{ t("vote.nominatedBy", { name: row.nominated_by ?? t("diary.deletedAccount") }) }}</span>
          </span>
        </RouterLink>

        <div class="vote-tally">
          <span class="tally-count nums">{{ row.votes }}</span>
          <span class="finer subtle">{{ tp("vote.voteWord", row.votes) }}</span>
          <span v-if="row.voters.length" class="finer subtle clamp-2">
            {{ row.voters.join(", ") }}
          </span>
        </div>

        <div class="vote-actions">
          <button
            class="btn btn-sm"
            :class="row.mine ? 'btn-primary' : 'btn-ghost'"
            type="button"
            :disabled="pendingId === row.id"
            @click="cast(row)"
          >
            {{ row.mine ? t("vote.yourVote") : t("vote.vote") }}
          </button>
          <button
            v-if="canManage"
            class="text-btn"
            type="button"
            :disabled="pendingId === row.id"
            @click="confirming = row"
          >
            {{ t("vote.confirmWinner") }}
          </button>
        </div>
      </li>
    </ol>

    <div v-else class="empty">
      <h3>{{ t("vote.noneTitle") }}</h3>
      <p>
        {{ t("vote.noneBody") }}
      </p>
      <div class="btn-row">
        <RouterLink class="btn btn-ghost" to="/discover">{{ t("home.findBook") }}</RouterLink>
      </div>
    </div>

    <p v-if="nominations.length" class="finer subtle vote-total nums">
      {{ tp("vote.cast", totalVotes) }}
    </p>

    <MeetingSheet
      v-if="confirming"
      :title="t('vote.confirmTitle')"
      :book-title="confirming.book.title"
      :timezone="vote?.timezone || 'UTC'"
      :confirm-label="t('vote.confirmWinner')"
      :blurb="t('vote.confirmBlurb')"
      @confirm="confirm"
      @close="confirming = null"
    />

    <BottomSheet v-if="editingDeadline" :title="t('vote.deadlineTitle')" @close="editingDeadline = false">
      <p class="fine subtle deadline-blurb">{{ t("vote.deadlineBlurb") }}</p>
      <label class="field">
        <span>{{ t("vote.deadlineLabel") }}</span>
        <input v-model="deadlineDraft" type="datetime-local" />
        <span class="field-hint">{{ t("meeting.optionalTz", { tz: vote?.timezone || "UTC" }) }}</span>
      </label>
      <div class="stack">
        <button
          class="btn btn-primary btn-block"
          type="button"
          @click="saveDeadline(deadlineDraft.trim() || null)"
        >
          {{ t("common.save") }}
        </button>
        <button
          v-if="vote?.closes_at"
          class="btn btn-ghost btn-block"
          type="button"
          @click="saveDeadline(null)"
        >
          {{ t("vote.clearDeadline") }}
        </button>
      </div>
    </BottomSheet>
  </section>
</template>

<style scoped>
.vote-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.vote-status {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1);
  margin-bottom: var(--space-4);
}

.deadline-blurb {
  margin-bottom: var(--space-4);
}

.skeleton-block {
  height: 120px;
}

.vote-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.vote-row {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: var(--space-3);
  align-items: start;
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.vote-row.leading {
  border-color: var(--accent-line);
}

.vote-book {
  display: flex;
  gap: var(--space-3);
  min-width: 0;
  color: inherit;
  text-decoration: none;
}

.vote-meta {
  display: grid;
  gap: 2px;
  align-content: start;
  min-width: 0;
}

.vote-meta strong {
  font-family: var(--serif);
  font-size: var(--text-md);
  line-height: var(--leading-snug);
}

.vote-tally {
  grid-column: 1;
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--space-1) var(--space-2);
}

.tally-count {
  font-family: var(--serif);
  font-size: var(--text-xl);
  font-weight: 700;
}

.vote-actions {
  grid-column: 2;
  grid-row: 1 / span 2;
  display: grid;
  justify-items: end;
  align-content: start;
  gap: var(--space-1);
}

.vote-total {
  margin-top: var(--space-3);
}

@media (min-width: 720px) {
  .vote-row {
    grid-template-columns: 1fr auto auto;
    align-items: center;
  }

  .vote-tally {
    grid-column: 2;
    flex-direction: column;
    align-items: flex-end;
    gap: 0;
  }

  .vote-actions {
    grid-column: 3;
    grid-row: 1;
  }
}
</style>
