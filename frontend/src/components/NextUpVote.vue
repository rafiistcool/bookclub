<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ApiError } from "../api/client";
import { countdown } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { useToast } from "../stores/toast";
import { useVote } from "../stores/vote";
import type { VoteNomination } from "../types";
import Avatar from "./Avatar.vue";
import BookCover from "./BookCover.vue";
import ClubPickSheet from "./ClubPickSheet.vue";
import NavIcon from "./NavIcon.vue";
import Sheet from "./Sheet.vue";

const emit = defineEmits<{
  applied: [];
}>();

const vote = useVote();
const flow = useFlow();
const toast = useToast();
const pendingId = ref<number | null>(null);
const closing = ref<VoteNomination | null>(null);
const deadlineOpen = ref(false);
const deadlineDraft = ref("");
const showSuggestions = ref(false);

const data = computed(() => vote.data);
const nominations = computed(() => data.value?.nominations ?? []);
const leader = computed(() => nominations.value.find((row) => row.id === data.value?.leader_id) ?? null);
const closes = computed(() => countdown(data.value?.closes_at));
const suggestions = computed(() => vote.suggestions.data ?? []);

async function cast(id: number) {
  pendingId.value = id;
  try {
    await vote.cast(id);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that vote");
  } finally {
    pendingId.value = null;
  }
}

async function apply(meetingAt: string | null) {
  const row = closing.value;
  closing.value = null;
  if (!row) return;
  pendingId.value = row.id;
  try {
    await vote.apply(row.id, meetingAt);
    toast.show(`“${row.book.title}” is the club pick`);
    emit("applied");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the winner");
  } finally {
    pendingId.value = null;
  }
}

async function saveDeadline(clear = false) {
  try {
    await vote.setDeadline(clear ? null : deadlineDraft.value || null);
    deadlineOpen.value = false;
    toast.show(clear ? "Deadline removed" : "Deadline set — the leader is applied automatically");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the deadline");
  }
}

function openDeadline() {
  deadlineDraft.value = data.value?.closes_local ?? "";
  deadlineOpen.value = true;
}

async function toggleSuggestions() {
  showSuggestions.value = !showSuggestions.value;
  if (showSuggestions.value) await vote.loadSuggestions();
}

onMounted(() => {
  void vote.load();
});
</script>

<template>
  <section class="section" aria-labelledby="next-up-title">
    <div class="section-title">
      <h2 id="next-up-title">Next up</h2>
      <button v-if="nominations.length" class="text-btn sm" type="button" @click="openDeadline">
        <NavIcon name="calendar" :size="16" />
        {{ data?.closes_at ? "Deadline" : "Set deadline" }}
      </button>
    </div>

    <p v-if="vote.error && !data" class="error fine">{{ vote.error }}</p>
    <div v-else-if="!data" class="list" aria-busy="true">
      <div v-for="n in 2" :key="n" class="nomination" style="border-color: transparent">
        <div class="skeleton cover" style="width: 44px" />
        <div style="display: grid; gap: 6px">
          <div class="skeleton line" style="width: 70%" />
          <div class="skeleton line" style="width: 40%; height: 0.7em" />
        </div>
      </div>
    </div>
    <template v-else>
      <div v-if="nominations.length" class="vote-meta" style="margin-bottom: 10px">
        <span>{{ data.voted_count }} / {{ data.member_count }} voted</span>
        <span v-if="data.not_voted.length" class="faint">
          Waiting on {{ data.not_voted.join(", ") }}
        </span>
        <span v-if="closes" :class="{ 'error': closes.past }">
          Closes {{ data.closes_label }} ({{ closes.label }})
        </span>
      </div>

      <p v-if="nominations.length === 0" class="muted fine">
        No nominations yet. Nominate from a book’s details, your shelf, or the shared to-read list.
      </p>
      <ol v-else class="list" style="list-style: none; padding: 0">
        <li
          v-for="row in nominations"
          :key="row.id"
          class="nomination"
          :class="{ leader: leader?.id === row.id && row.votes > 0 }"
        >
          <button type="button" style="all: unset; cursor: pointer" :aria-label="`Details for ${row.book.title}`" @click="flow.open({ kind: 'details', book: toRef(row.book) })">
            <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="sm" />
          </button>
          <div class="body">
            <h3 class="clamp-1">{{ row.book.title }}</h3>
            <p class="fine muted clamp-1">{{ row.book.authors }}</p>
            <p class="fine" style="display: flex; align-items: center; gap: 6px; margin-top: 4px">
              <span class="avatar-stack" v-if="row.voters.length">
                <Avatar v-for="name in row.voters" :key="name" :username="name" size="sm" />
              </span>
              <span :class="row.votes ? 'strong' : 'faint'">{{ row.votes }} {{ row.votes === 1 ? "vote" : "votes" }}</span>
              <span class="faint">· {{ row.nominated_by }}</span>
            </p>
          </div>
          <button
            class="btn vote-btn"
            :class="row.mine ? 'btn-primary' : 'btn-ghost'"
            type="button"
            :aria-pressed="row.mine"
            :disabled="pendingId === row.id"
            @click="cast(row.id)"
          >
            <NavIcon v-if="row.mine" name="check" :size="18" />
            {{ row.mine ? "Voted" : "Vote" }}
          </button>
        </li>
      </ol>

      <div v-if="nominations.length" class="actions" style="margin-top: 12px; justify-content: space-between">
        <span class="fine faint">{{ nominations.length }} / {{ data.nomination_limit }} nominated</span>
        <button
          v-if="leader && leader.votes > 0"
          class="btn btn-ghost btn-sm"
          type="button"
          :disabled="pendingId !== null"
          @click="closing = leader"
        >
          Close vote · pick “{{ leader.book.title }}”
        </button>
      </div>

      <div style="margin-top: 14px">
        <button class="text-btn sm" type="button" :aria-expanded="showSuggestions" @click="toggleSuggestions">
          {{ showSuggestions ? "Hide suggestions" : "Suggestions for the vote" }}
        </button>
        <div v-if="showSuggestions" class="card" style="margin-top: 6px; padding: 4px 12px">
          <p v-if="vote.suggestions.pending && !suggestions.length" class="muted fine" style="padding: 8px 0">Thinking…</p>
          <p v-else-if="suggestions.length === 0" class="muted fine" style="padding: 8px 0">
            Nothing to suggest yet — add books to Want to read and finish a pick or two.
          </p>
          <div v-for="row in suggestions" :key="row.book.ol_work_key" class="suggestion">
            <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="xs" />
            <div style="min-width: 0">
              <p class="strong clamp-1">{{ row.book.title }}</p>
              <p class="fine muted clamp-1">{{ row.reasons.join(" · ") }}</p>
            </div>
            <button class="btn btn-ghost btn-sm" type="button" :disabled="!data.can_nominate" @click="flow.nominate(toRef(row.book))">
              Nominate
            </button>
          </div>
        </div>
      </div>
    </template>

    <ClubPickSheet
      v-if="closing"
      title="Close the vote"
      :book-title="closing.book.title"
      :timezone="data?.timezone || 'UTC'"
      confirm-label="Confirm winner"
      hide-note
      @confirm="(meetingAt) => apply(meetingAt)"
      @close="closing = null"
    />

    <Sheet v-if="deadlineOpen" title="Vote deadline" subtitle="The leading book becomes the club pick when the deadline passes." @close="deadlineOpen = false">
      <label class="field">
        <span>Closes <span class="faint">({{ data?.timezone }})</span></span>
        <input v-model="deadlineDraft" type="datetime-local" data-autofocus />
      </label>
      <template #foot>
        <button class="btn btn-primary" type="button" :disabled="!deadlineDraft" @click="saveDeadline()">Save deadline</button>
        <button v-if="data?.closes_at" class="btn btn-ghost" type="button" @click="saveDeadline(true)">Remove deadline</button>
      </template>
    </Sheet>
  </section>
</template>
