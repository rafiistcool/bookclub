<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import ClubPickSheet from "../components/ClubPickSheet.vue";
import FinishNoteSheet from "../components/FinishNoteSheet.vue";
import NextUpVote from "../components/NextUpVote.vue";
import ProgressSheet from "../components/ProgressSheet.vue";
import PickThread from "../components/PickThread.vue";
import StatusSheet from "../components/StatusSheet.vue";
import type { FinishNote, Status } from "../constants";
import { STATUS_LABEL, starLabel } from "../constants";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { ClubPick, ClubPickBook } from "../types";

const pick = ref<ClubPick | null>(null);
const history = ref<ClubPick[]>([]);
const timezone = ref("UTC");
const error = ref("");
const loaded = ref(false);
const adding = ref(false);
const moving = ref(false);
const finishing = ref<{
  mode: "add" | "move";
  status: "finished" | "did_not_finish";
} | null>(null);
const editingMeeting = ref(false);
const editingProgress = ref(false);
const openPast = ref<number | null>(null);
const toast = useToast();
const session = useSession();

const reading = computed(() =>
  (pick.value?.readers ?? []).filter((row) => row.status === "currently_reading"),
);
const myProgress = computed(() => {
  const current = pick.value;
  const username = session.user?.username;
  if (!current || current.on_shelf !== "currently_reading" || !username) return null;
  return current.readers.find((row) => row.username === username)?.progress ?? null;
});
const finished = computed(() =>
  (pick.value?.readers ?? []).filter((row) => row.status === "finished"),
);

async function load() {
  try {
    const [current, past] = await Promise.all([api.clubPick(), api.clubPickHistory()]);
    pick.value = current.pick;
    timezone.value = current.timezone;
    history.value = past.items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load the club pick";
  } finally {
    loaded.value = true;
  }
}

async function add(status: Status, note: FinishNote = {}) {
  const current = pick.value;
  adding.value = false;
  if (!current) return;
  try {
    const created = await api.addToShelf({
      ol_work_key: current.book.ol_work_key,
      title: current.book.title,
      authors: current.book.authors,
      cover_id: current.book.cover_id,
      year: current.book.year,
      status,
      ...note,
    });
    current.on_shelf = status;
    current.shelf_id = created.id;
    toast.show(`Added to ${STATUS_LABEL[status]}`);
    await load();
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      current.on_shelf = err.item.status;
      current.shelf_id = err.item.id;
      moving.value = true;
      return;
    }
    toast.show(err instanceof ApiError ? err.message : "Could not add that book");
  }
}

async function moveTo(status: Status, note: FinishNote = {}) {
  const current = pick.value;
  moving.value = false;
  if (!current?.shelf_id) return;
  try {
    await api.patchShelf(current.shelf_id, { status, position: 0, ...note });
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
    await load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not move that book");
  }
}

async function saveProgress(progress: number | null) {
  const current = pick.value;
  editingProgress.value = false;
  if (!current?.shelf_id) return;
  try {
    await api.patchShelf(current.shelf_id, { progress });
    toast.show(progress == null ? "Progress cleared" : `Progress: ${progress}%`);
    await load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save progress");
  }
}

async function saveMeeting(meetingAt: string | null) {
  const current = pick.value;
  editingMeeting.value = false;
  if (!current) return;
  const body: ClubPickBook = {
    ol_work_key: current.book.ol_work_key,
    title: current.book.title,
    authors: current.book.authors,
    cover_id: current.book.cover_id,
    year: current.book.year,
    meeting_at: meetingAt,
  };
  try {
    pick.value = await api.setClubPick(body);
    toast.show(meetingAt ? "Meeting saved" : "Meeting cleared");
    await load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not update the meeting");
  }
}

onMounted(load);
</script>

<template>
  <section class="home-page">
    <h1>Home</h1>
    <p class="lede">This month’s club pick — one book for everyone.</p>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <template v-else>
      <article v-if="pick" class="home-pick">
        <div class="home-cover">
          <BookCover :title="pick.book.title" :cover-id="pick.book.cover_id" size="L" fluid />
        </div>
        <div class="book-meta">
          <p class="club-pick-kicker">Club pick</p>
          <h2>{{ pick.book.title }}</h2>
          <p v-if="pick.book.authors">{{ pick.book.authors }}</p>
          <p v-if="pick.book.year" class="fine">{{ pick.book.year }}</p>
          <p class="fine muted">Chosen by {{ pick.set_by }}</p>
          <p v-if="pick.meeting_label" class="club-pick-meeting">{{ pick.meeting_label }}</p>
          <p v-else class="fine muted">No meeting set.</p>
        </div>
        <div class="club-pick-actions">
          <button v-if="!pick.on_shelf" class="btn btn-primary" type="button" @click="adding = true">
            Add to shelf
          </button>
          <button v-else class="btn btn-primary" type="button" @click="moving = true">
            Move to…
          </button>
          <button class="btn btn-ghost" type="button" @click="editingMeeting = true">
            {{ pick.meeting_at ? "Change meeting" : "Add meeting" }}
          </button>
          <button
            v-if="pick.on_shelf === 'currently_reading'"
            class="btn btn-ghost"
            type="button"
            @click="editingProgress = true"
          >
            {{ myProgress == null ? "Set progress" : `Progress: ${myProgress}%` }}
          </button>
        </div>
        <PickThread :pick-id="pick.id" />
      </article>
      <div v-else class="empty">
        No club pick yet.
        <RouterLink to="/library">Choose a book in the library</RouterLink>
        or from
        <RouterLink to="/shelf">your shelf</RouterLink>.
      </div>

      <div v-if="pick" class="home-progress">
        <section>
          <h2>Reading</h2>
          <p v-if="reading.length === 0" class="muted fine">Nobody yet.</p>
          <ul v-else>
            <li v-for="row in reading" :key="row.username">
              <RouterLink :to="`/friends/${row.username}`">{{ row.username }}</RouterLink>
              <p v-if="row.progress != null" class="finish-note">{{ row.progress }}%</p>
            </li>
          </ul>
        </section>
        <section>
          <h2>Finished</h2>
          <p v-if="finished.length === 0" class="muted fine">Nobody yet.</p>
          <ul v-else>
            <li v-for="row in finished" :key="row.username">
              <RouterLink :to="`/friends/${row.username}`">{{ row.username }}</RouterLink>
              <p v-if="row.rating" class="finish-note">{{ starLabel(row.rating) }}</p>
              <p v-if="row.take" class="finish-note">{{ row.take }}</p>
            </li>
          </ul>
        </section>
      </div>

      <NextUpVote @applied="load" />

      <section v-if="history.length" class="club-pick-history">
        <h2>Past picks</h2>
        <div class="book-list">
          <article v-for="row in history" :key="row.id" class="past-pick">
            <button class="book-card compact past-pick-toggle" type="button" @click="openPast = openPast === row.id ? null : row.id">
              <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="S" />
              <div class="book-meta">
                <h3>{{ row.book.title }}</h3>
                <p class="fine muted">{{ row.set_by }}</p>
                <p v-if="row.meeting_label" class="fine muted">{{ row.meeting_label }}</p>
                <p class="fine">{{ openPast === row.id ? "Hide notes" : "Notes" }}</p>
              </div>
            </button>
            <PickThread v-if="openPast === row.id" :pick-id="row.id" :can-post="false" />
          </article>
        </div>
      </section>
    </template>

    <StatusSheet
      v-if="adding"
      title="Add the club pick"
      current="currently_reading"
      @pick="add"
      @finish="(status) => { adding = false; finishing = { mode: 'add', status } }"
      @close="adding = false"
    />
    <StatusSheet
      v-if="moving && pick"
      title="Move the club pick"
      :current="pick.on_shelf"
      @pick="moveTo"
      @finish="(status) => { moving = false; finishing = { mode: 'move', status } }"
      @close="moving = false"
    />
    <FinishNoteSheet
      v-if="finishing && pick"
      :title="finishing.status === 'finished' ? 'Finished' : 'Did not finish'"
      :book-title="pick.book.title"
      :status="finishing.status"
      @confirm="(note) => { const next = finishing; finishing = null; if (next?.mode === 'add') void add(next.status, note); else if (next) void moveTo(next.status, note); }"
      @close="finishing = null"
    />
    <ProgressSheet
      v-if="editingProgress && pick"
      title="Reading progress"
      :book-title="pick.book.title"
      :progress="myProgress"
      @confirm="saveProgress"
      @close="editingProgress = false"
    />
    <ClubPickSheet
      v-if="editingMeeting && pick"
      title="Meeting"
      :book-title="pick.book.title"
      :timezone="timezone"
      :meeting-local="pick.meeting_local"
      confirm-label="Save meeting"
      @confirm="saveMeeting"
      @close="editingMeeting = false"
    />
  </section>
</template>
