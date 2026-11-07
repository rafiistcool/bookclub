<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { formatEventTime } from "../i18n/dates";
import { tp } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
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
const session = useSession();

const vote = ref<NextUpVoteState | null>(null);
const error = ref("");
const loaded = ref(false);
const pendingId = ref<number | null>(null);
const confirming = ref<VoteNomination | null>(null);
const choosing = ref(false);
const editingDeadline = ref(false);
const deadlineDraft = ref("");

// The server's order (votes, then the earlier nomination) and its leader,
// so the row marked as leading is the book the deadline would make the pick.
const nominations = computed(() => vote.value?.nominations ?? []);

function share(row: VoteNomination): number {
  const members = Math.max(1, vote.value?.member_count ?? 1);
  return Math.min(1, row.votes / members);
}


const closes = computed(() =>
  vote.value?.closes_at
    ? formatEventTime(vote.value.closes_at, vote.value.timezone, locale.locale) || vote.value.closes_label
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

/** The ballot as it will be once the server agrees, so the bars move under the finger. */
function withBallot(state: NextUpVoteState, id: number): NextUpVoteState {
  const me = session.user?.username ?? "";
  const nominations = state.nominations
    .map((row) => {
      if (row.id === id && !row.mine) return { ...row, mine: true, votes: row.votes + 1, voters: [...row.voters, me].sort() };
      if (row.id !== id && row.mine) return { ...row, mine: false, votes: row.votes - 1, voters: row.voters.filter((name) => name !== me) };
      return row;
    })
    .sort((a, b) => b.votes - a.votes || a.id - b.id);
  const first = state.my_vote_id == null;
  // Like the server: the leader needs a vote, and a tie stays with the
  // earlier nomination, which the sort puts first.
  const top = nominations[0];
  return {
    ...state,
    nominations,
    leader_id: top && top.votes > 0 ? top.id : null,
    my_vote_id: id,
    voted_count: state.voted_count + (first ? 1 : 0),
    not_voted: state.not_voted.filter((name) => name !== me),
  };
}

async function cast(row: VoteNomination) {
  const before = vote.value;
  if (!before || row.mine) return;
  pendingId.value = row.id;
  vote.value = withBallot(before, row.id);
  if (typeof navigator !== "undefined" && typeof navigator.vibrate === "function") navigator.vibrate(10);
  try {
    vote.value = await api.castVote(props.slug, row.id);
  } catch (err) {
    vote.value = before;
    fail(err, t("vote.saveFailed"));
  } finally {
    pendingId.value = null;
  }
}

function choose(row: VoteNomination) {
  choosing.value = false;
  confirming.value = row;
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
      <span class="vote-fact nums">{{ t("vote.votedOf", { voted: vote.voted_count, total: vote.member_count }) }}</span>
      <span v-if="vote.not_voted.length" class="vote-fact subtle">
        {{ tp("vote.waitingFor", vote.not_voted.length, { names: vote.not_voted.join(", ") }) }}
      </span>
      <span v-if="closes" class="vote-fact">{{ t("vote.closesAt", { when: closes }) }}</span>
      <button v-if="canManage" class="text-btn" type="button" @click="openDeadline">
        {{ vote.closes_at ? t("vote.changeDeadline") : t("vote.setDeadline") }}
      </button>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton skeleton-block" aria-hidden="true" />

    <template v-else-if="nominations.length">
    <TransitionGroup tag="ol" name="vote" class="vote-list">
      <li
        v-for="row in nominations"
        :key="row.id"
        class="vote-row"
        :class="{ leading: row.id === vote?.leader_id, mine: row.mine }"
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
          <span class="tally-bar" aria-hidden="true">
            <span class="tally-fill" :style="{ transform: `scaleX(${share(row)})` }" />
          </span>
          <span class="tally-line">
            <span class="tally-number">
              <Transition name="count">
                <span :key="row.votes" class="tally-count nums">{{ row.votes }}</span>
              </Transition>
            </span>
            <span class="finer subtle">{{ tp("vote.voteWord", row.votes) }}</span>
            <span v-if="row.voters.length" class="finer subtle tally-voters" :title="row.voters.join(', ')">{{ row.voters.join(", ") }}</span>
          </span>
        </div>

        <div class="vote-actions">
          <button
            class="btn btn-sm vote-btn"
            :class="row.mine ? 'btn-primary' : 'btn-ghost'"
            type="button"
            :aria-pressed="row.mine"
            :disabled="pendingId !== null"
            @click="cast(row)"
          >
            <svg v-if="row.mine" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M5 12.5 10 17l9-10" />
            </svg>
            {{ row.mine ? t("vote.yourVote") : t("vote.vote") }}
          </button>
        </div>
      </li>
    </TransitionGroup>

    <button
      v-if="canManage"
      class="btn btn-ghost btn-block confirm-winner"
      type="button"
      :disabled="pendingId !== null"
      @click="choosing = true"
    >
      {{ t("vote.confirmWinner") }}
    </button>
    </template>

    <div v-else class="empty">
      <h3>{{ t("vote.noneTitle") }}</h3>
      <p>
        {{ t("vote.noneBody") }}
      </p>
      <div class="btn-row">
        <RouterLink class="btn btn-ghost" to="/discover">{{ t("home.findBook") }}</RouterLink>
      </div>
    </div>


    <BottomSheet v-if="choosing" :title="t('vote.chooseWinner')" @close="choosing = false">
      <div class="stack">
        <button
          v-for="row in nominations"
          :key="row.id"
          class="winner-choice"
          :class="{ leading: row.id === vote?.leader_id }"
          type="button"
          @click="choose(row)"
        >
          <BookCover :book="row.book" size="xs" />
          <span class="choice-title">{{ row.book.title }}</span>
          <span class="choice-votes nums">{{ row.votes }}</span>
        </button>
      </div>
    </BottomSheet>

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

/* As flex items the facts wrap as units, and the dot between two facts stays
   at the end of a line instead of starting the next one. */
.vote-status {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 0.3em;
  margin-bottom: var(--space-4);
}

.vote-fact:has(+ .vote-fact)::after {
  content: " ·";
}

.vote-status .text-btn {
  margin-inline-start: var(--space-1);
}

.deadline-blurb {
  margin-bottom: var(--space-4);
}

.skeleton-block {
  height: 120px;
}

/* Light rows split by hairlines. The action column has one width, so every
   tally bar starts and ends at the same place whatever its button says. */
.vote-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.vote-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 9.5rem;
  grid-template-areas:
    "book book"
    "tally action";
  gap: var(--space-3);
  align-items: center;
  padding: var(--space-3) 0;
}

.vote-row + .vote-row {
  border-top: 1px solid var(--border);
}

/* The leader is marked by its accent tally and title, not a box. */
.vote-row.leading .vote-meta strong {
  color: var(--accent);
}

/* A new leader climbs past the others instead of the list jumping. */
.vote-move {
  transition: transform var(--dur-slow) var(--ease-out);
}

.vote-book {
  grid-area: book;
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
  grid-area: tally;
  display: grid;
  gap: var(--space-1);
  min-width: 0;
}

/* The share of the club behind a book: members, not ballots, so an empty
   bar still says how many are missing. */
.tally-bar {
  position: relative;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  overflow: hidden;
}

.tally-fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: var(--text-subtle);
  transform-origin: left center;
  transition: transform 600ms var(--ease-out);
}

.vote-row.leading .tally-fill,
.vote-row.mine .tally-fill {
  background: var(--accent);
}

.tally-line {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-2);
  min-width: 0;
}

.tally-count {
  display: inline-block;
  font-family: var(--serif);
  font-size: var(--text-xl);
  font-weight: 700;
  line-height: 1;
}

.tally-voters {
  flex-basis: 100%;
  min-width: 0;
  overflow: hidden;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
}

/* The count rolls: the old number lifts away while the new one rises in
   under it. */
.tally-number {
  position: relative;
  display: inline-block;
}

.count-leave-active {
  position: absolute;
  left: 0;
  top: 0;
}

.count-enter-active,
.count-leave-active {
  transition:
    transform var(--dur-fast) var(--ease-out),
    opacity var(--dur-fast) var(--ease);
}

.count-enter-from {
  opacity: 0;
  transform: translateY(50%);
}

.count-leave-to {
  opacity: 0;
  transform: translateY(-50%);
}

.vote-actions {
  grid-area: action;
}

.vote-btn {
  width: 100%;
  min-width: 0;
}

.confirm-winner {
  margin-top: var(--space-4);
}

.winner-choice {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: var(--space-3);
  min-height: 56px;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: inherit;
  text-align: left;
  transition: transform var(--dur-fast) var(--ease-out);
}

.winner-choice.leading {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.winner-choice:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.choice-title {
  font-family: var(--serif);
  font-weight: 600;
  min-width: 0;
}

.choice-votes {
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 700;
}

@media (min-width: 720px) {
  .vote-row {
    grid-template-columns: minmax(0, 1fr) 200px 9.5rem;
    grid-template-areas: "book tally action";
  }
}
</style>
