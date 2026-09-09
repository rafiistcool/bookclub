<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { useToast } from "../stores/toast";
import type { NextUpVote as NextUpVoteState, VoteNomination } from "../types";
import BookCover from "./BookCover.vue";
import MeetingSheet from "./MeetingSheet.vue";

const emit = defineEmits<{ applied: [] }>();

const vote = ref<NextUpVoteState | null>(null);
const error = ref("");
const loaded = ref(false);
const pendingId = ref<number | null>(null);
const confirming = ref<VoteNomination | null>(null);
const toast = useToast();

const nominations = computed(() =>
  [...(vote.value?.nominations ?? [])].sort(
    (a, b) => b.votes - a.votes || a.book.title.localeCompare(b.book.title),
  ),
);

const totalVotes = computed(() =>
  nominations.value.reduce((sum, row) => sum + row.votes, 0),
);

async function load() {
  try {
    vote.value = await api.nextUp();
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load the vote";
  } finally {
    loaded.value = true;
  }
}

async function cast(row: VoteNomination) {
  pendingId.value = row.id;
  try {
    vote.value = await api.castVote(row.id);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that vote");
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
    const result = await api.applyWinner(row.id, meetingAt);
    vote.value = result.vote;
    toast.show(`“${row.book.title}” is now the club pick`);
    emit("applied");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not confirm that winner");
  } finally {
    pendingId.value = null;
  }
}

onMounted(load);
defineExpose({ load });
</script>

<template>
  <section aria-labelledby="next-up-heading">
    <div class="section-head">
      <h2 id="next-up-heading">Next-up vote</h2>
      <span v-if="vote" class="fine subtle nums">
        {{ nominations.length }} of {{ vote.nomination_limit }} nominations
      </span>
    </div>
    <p class="fine muted vote-blurb">
      Everyone gets one vote and can change it any time — voting does not change the
      current pick. When you're ready, one of you confirms the winner, which replaces
      the pick for the whole club.
    </p>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton skeleton-block" aria-hidden="true" />

    <ol v-else-if="nominations.length" class="vote-list">
      <li
        v-for="(row, index) in nominations"
        :key="row.id"
        class="vote-row"
        :class="{ leading: index === 0 && row.votes > 0 }"
      >
        <RouterLink class="vote-book" :to="bookPath(row.book.ol_work_key)">
          <BookCover
            :title="row.book.title"
            :cover-id="row.book.cover_id"
            :work-key="row.book.ol_work_key"
            size="sm"
          />
          <span class="vote-meta">
            <strong>{{ row.book.title }}</strong>
            <span v-if="row.book.authors" class="finer subtle">
              {{ row.book.authors }}
            </span>
            <span class="finer subtle">Nominated by {{ row.nominated_by }}</span>
          </span>
        </RouterLink>

        <div class="vote-tally">
          <span class="tally-count nums">{{ row.votes }}</span>
          <span class="finer subtle">{{ row.votes === 1 ? "vote" : "votes" }}</span>
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
            {{ row.mine ? "Your vote" : "Vote" }}
          </button>
          <button
            class="text-btn"
            type="button"
            :disabled="pendingId === row.id"
            @click="confirming = row"
          >
            Confirm winner
          </button>
        </div>
      </li>
    </ol>

    <div v-else class="empty">
      <h3>No nominations yet</h3>
      <p>
        Open any book and choose “Nominate for next up”. Books more than one of you
        wants to read are a good place to start.
      </p>
      <div class="btn-row">
        <RouterLink class="btn btn-ghost" to="/discover">Find a book</RouterLink>
      </div>
    </div>

    <p v-if="nominations.length" class="finer subtle vote-total nums">
      {{ totalVotes }} {{ totalVotes === 1 ? "vote" : "votes" }} cast
    </p>

    <MeetingSheet
      v-if="confirming"
      title="Confirm as the club pick"
      :book-title="confirming.book.title"
      :timezone="vote?.timezone || 'UTC'"
      confirm-label="Confirm winner"
      blurb="This ends the vote and replaces the current pick for everyone."
      @confirm="confirm"
      @close="confirming = null"
    />
  </section>
</template>

<style scoped>
.vote-blurb {
  max-width: 62ch;
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
