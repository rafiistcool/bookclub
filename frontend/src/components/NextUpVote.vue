<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import type { NextUpVote as NextUpVoteState, VoteNomination } from "../types";
import BookCover from "./BookCover.vue";
import ClubPickSheet from "./ClubPickSheet.vue";

const emit = defineEmits<{
  applied: [];
}>();

const vote = ref<NextUpVoteState | null>(null);
const error = ref("");
const loaded = ref(false);
const pendingId = ref<number | null>(null);
const applying = ref<VoteNomination | null>(null);
const toast = useToast();

const empty = computed(() => (vote.value?.nominations.length ?? 0) === 0);

async function load() {
  try {
    vote.value = await api.nextUp();
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load next up";
  } finally {
    loaded.value = true;
  }
}

async function cast(id: number) {
  pendingId.value = id;
  try {
    vote.value = await api.castVote(id);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that vote");
  } finally {
    pendingId.value = null;
  }
}

async function apply(meetingAt: string | null) {
  const row = applying.value;
  applying.value = null;
  if (!row) return;
  pendingId.value = row.id;
  try {
    const result = await api.applyWinner(row.id, meetingAt);
    vote.value = result.vote;
    toast.show(`Set ${row.book.title} as the club pick`);
    emit("applied");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the winner");
  } finally {
    pendingId.value = null;
  }
}

onMounted(load);

defineExpose({ load });
</script>

<template>
  <section class="next-up">
    <h2>Next up</h2>
    <p class="muted fine">
      Nominate a book, one vote each. Confirm a winner when you’re ready — it won’t replace the
      current pick until then.
    </p>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <p v-else-if="empty" class="muted fine">
      No nominations yet. Add one from
      <RouterLink to="/shelf">your shelf</RouterLink>,
      <RouterLink to="/overlap">TBR overlap</RouterLink>, or
      <RouterLink to="/library">the library</RouterLink>.
    </p>
    <ol v-else class="next-up-list">
      <li v-for="row in vote?.nominations" :key="row.id" class="next-up-row">
        <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="S" />
        <div class="book-meta">
          <h3>{{ row.book.title }}</h3>
          <p v-if="row.book.authors" class="fine muted">{{ row.book.authors }}</p>
          <p class="fine">
            {{ row.votes }} {{ row.votes === 1 ? "vote" : "votes" }}
            <span v-if="row.voters.length" class="muted"> · {{ row.voters.join(", ") }}</span>
          </p>
          <p class="fine muted">Nominated by {{ row.nominated_by }}</p>
        </div>
        <div class="next-up-actions">
          <button
            class="btn"
            :class="row.mine ? 'btn-primary' : 'btn-ghost'"
            type="button"
            :disabled="pendingId === row.id"
            @click="cast(row.id)"
          >
            {{ row.mine ? "Your vote" : "Vote" }}
          </button>
          <button
            class="btn btn-ghost"
            type="button"
            :disabled="pendingId === row.id"
            @click="applying = row"
          >
            Set as club pick
          </button>
        </div>
      </li>
    </ol>
    <p v-if="vote && !empty" class="fine muted">
      {{ vote.nominations.length }} / {{ vote.nomination_limit }} nominations
    </p>
    <ClubPickSheet
      v-if="applying"
      title="Set winner as club pick"
      :book-title="applying.book.title"
      :timezone="vote?.timezone || 'UTC'"
      confirm-label="Confirm winner"
      @confirm="apply"
      @close="applying = null"
    />
  </section>
</template>
