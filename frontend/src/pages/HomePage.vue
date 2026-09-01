<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import ActivityFeed from "../components/ActivityFeed.vue";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import ClubPickSheet from "../components/ClubPickSheet.vue";
import MilestoneList from "../components/MilestoneList.vue";
import NavIcon from "../components/NavIcon.vue";
import NextUpVote from "../components/NextUpVote.vue";
import PickThread from "../components/PickThread.vue";
import ProgressBar from "../components/ProgressBar.vue";
import Skeleton from "../components/Skeleton.vue";
import StarRating from "../components/StarRating.vue";
import { STATUS_SHORT, countdown } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useSession } from "../stores/session";
import { useShelf } from "../stores/shelf";
import { useToast } from "../stores/toast";
import type { Milestone } from "../types";

const pickStore = usePick();
const shelf = useShelf();
const flow = useFlow();
const session = useSession();
const toast = useToast();

const editingMeeting = ref(false);
const milestones = ref<Milestone[]>([]);
const openPast = ref<number | null>(null);
const feed = ref<InstanceType<typeof ActivityFeed> | null>(null);

const pick = computed(() => pickStore.pick);
const history = computed(() => pickStore.history.data ?? []);
const meeting = computed(() => countdown(pick.value?.meeting_at));
const myItem = computed(() => (pick.value ? shelf.byKey.get(pick.value.book.ol_work_key) ?? null : null));
const readers = computed(() => pick.value?.readers ?? []);
const others = computed(() => readers.value.filter((row) => row.username !== session.user?.username));
const mine = computed(() => readers.value.find((row) => row.username === session.user?.username) ?? null);

async function loadMilestones() {
  if (!pick.value) return;
  try {
    milestones.value = (await api.milestones(pick.value.id)).items;
  } catch {
    milestones.value = [];
  }
}

async function load(force = false) {
  await Promise.all([pickStore.load(force), pickStore.loadHistory(force), shelf.load(force)]);
  await loadMilestones();
}

async function saveMeeting(meetingAt: string | null, note: string) {
  const current = pick.value;
  editingMeeting.value = false;
  if (!current) return;
  try {
    await pickStore.set({ ...toRef(current.book), meeting_at: meetingAt, note });
    toast.show(meetingAt ? "Meeting saved" : "Meeting cleared");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not update the meeting");
  }
}

function primaryAction() {
  const current = pick.value;
  if (!current) return;
  const book = toRef(current.book);
  const item = myItem.value;
  if (!item) {
    flow.open({ kind: "status", book, item: null, title: "Add the club pick" });
    return;
  }
  if (item.status === "currently_reading") {
    flow.open({ kind: "progress", item });
    return;
  }
  flow.open({ kind: "status", book, item, title: "Move the club pick" });
}

const primaryLabel = computed(() => {
  const item = myItem.value;
  if (!item) return "Start reading";
  if (item.status === "currently_reading") return item.progress == null ? "Set progress" : `Update progress · ${item.progress}%`;
  return `In ${STATUS_SHORT[item.status]} · Move`;
});

onMounted(() => {
  void load();
});
</script>

<template>
  <section aria-label="Club pick">
    <p v-if="pickStore.error && !pickStore.loaded" class="error">{{ pickStore.error }}</p>
    <Skeleton v-else-if="!pickStore.loaded" kind="hero" />

    <template v-else-if="pick">
      <article class="pick-hero">
        <button type="button" style="all: unset; cursor: pointer" aria-label="Book details" @click="flow.open({ kind: 'details', book: toRef(pick.book) })">
          <BookCover :title="pick.book.title" :cover-id="pick.book.cover_id" size="lg" eager />
        </button>
        <div class="meta">
          <p class="kicker">Club pick</p>
          <h1>{{ pick.book.title }}</h1>
          <p class="muted fine clamp-1">{{ pick.book.authors }}<template v-if="pick.book.year"> · {{ pick.book.year }}</template></p>
          <p v-if="meeting" class="fine countdown">
            <NavIcon name="calendar" :size="16" />
            <span :class="{ soon: meeting.soon }">Meeting {{ meeting.label }}</span>
            <span class="faint">· {{ pick.meeting_label }}</span>
          </p>
          <p v-else class="fine faint">No meeting yet</p>
          <p v-if="pick.note" class="fine muted serif" style="font-style: italic">“{{ pick.note }}” <span class="faint" style="font-style: normal">— {{ pick.set_by }}</span></p>
          <p v-else class="tiny faint">Chosen by {{ pick.set_by }}</p>
        </div>
        <div class="actions">
          <button class="btn btn-primary" type="button" @click="primaryAction">{{ primaryLabel }}</button>
          <button class="btn btn-ghost btn-sm" type="button" @click="editingMeeting = true">
            {{ pick.meeting_at ? "Change meeting" : "Add meeting" }}
          </button>
          <a v-if="pick.meeting_at" class="btn btn-ghost btn-sm" :href="api.meetingIcsUrl()" download>
            <NavIcon name="calendar" :size="16" /> Add to calendar
          </a>
        </div>
      </article>

      <section class="section" aria-labelledby="readers-title">
        <div class="section-title">
          <h2 id="readers-title">Where everyone is</h2>
        </div>
        <div class="readers">
          <div v-if="mine" class="reader-row">
            <Avatar :username="mine.username" size="sm" />
            <div style="min-width: 0">
              <span class="name">You</span>
              <span class="sub"> · {{ STATUS_SHORT[mine.status] }}</span>
              <ProgressBar v-if="mine.status === 'currently_reading'" :value="mine.progress" thin style="margin-top: 4px" />
            </div>
            <StarRating v-if="mine.rating" :value="mine.rating" />
            <p v-if="mine.take" class="take">“{{ mine.take }}”</p>
          </div>
          <div v-for="row in others" :key="row.username" class="reader-row">
            <Avatar :username="row.username" size="sm" />
            <div style="min-width: 0">
              <RouterLink class="name" :to="`/friends/${row.username}`">{{ row.username }}</RouterLink>
              <span class="sub"> · {{ STATUS_SHORT[row.status] }}</span>
              <ProgressBar v-if="row.status === 'currently_reading'" :value="row.progress" thin style="margin-top: 4px" />
            </div>
            <StarRating v-if="row.rating" :value="row.rating" />
            <p v-if="row.take" class="take">“{{ row.take }}”</p>
            <p v-else-if="row.dnf_reason" class="take">{{ row.dnf_reason }}</p>
          </div>
          <p v-if="readers.length === 0" class="muted fine">Nobody has shelved it yet — you could be first.</p>
        </div>
      </section>

      <div class="two-col">
        <div>
          <MilestoneList
            :pick-id="pick.id"
            :milestones="milestones"
            :timezone="pickStore.timezone"
            :can-edit="true"
            @changed="loadMilestones"
          />
        </div>
        <section class="section" aria-labelledby="notes-title">
          <div class="section-title">
            <h2 id="notes-title">Notes</h2>
            <span class="fine faint">Flag spoilers by progress</span>
          </div>
          <PickThread :pick-id="pick.id" :milestones="milestones" @changed="loadMilestones" />
        </section>
      </div>
    </template>

    <div v-else class="empty">
      <p class="kicker">No club pick yet</p>
      <p>Choose a book in the <RouterLink to="/library">library</RouterLink>, from <RouterLink to="/shelf">your shelf</RouterLink>, or run a vote below.</p>
    </div>

    <NextUpVote @applied="load(true)" />

    <section class="section" aria-labelledby="activity-title">
      <div class="section-title">
        <h2 id="activity-title">Lately</h2>
        <RouterLink class="fine" to="/friends">Everything</RouterLink>
      </div>
      <ActivityFeed ref="feed" compact :limit="6" />
    </section>

    <section v-if="history.length" class="section" aria-labelledby="history-title">
      <div class="section-title">
        <h2 id="history-title">Past picks</h2>
        <RouterLink class="fine" to="/stats">Year in review</RouterLink>
      </div>
      <div class="list">
        <article v-for="row in history" :key="row.id" class="card" style="padding: 10px 12px">
          <div class="row-item plain" style="padding: 0">
            <button type="button" style="all: unset; cursor: pointer" @click="flow.open({ kind: 'details', book: toRef(row.book) })">
              <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="sm" />
            </button>
            <div class="row-body">
              <h3 class="clamp-1">{{ row.book.title }}</h3>
              <p class="clamp-1">{{ row.set_by }}<template v-if="row.meeting_label"> · {{ row.meeting_label }}</template></p>
            </div>
            <button class="text-btn sm" type="button" :aria-expanded="openPast === row.id" @click="openPast = openPast === row.id ? null : row.id">
              {{ openPast === row.id ? "Hide notes" : "Notes" }}
            </button>
          </div>
          <div v-if="openPast === row.id" style="margin-top: 10px">
            <PickThread :pick-id="row.id" read-only compact />
          </div>
        </article>
      </div>
    </section>

    <ClubPickSheet
      v-if="editingMeeting && pick"
      title="Meeting"
      :book-title="pick.book.title"
      :timezone="pickStore.timezone"
      :meeting-local="pick.meeting_local"
      :note="pick.note"
      confirm-label="Save"
      @confirm="saveMeeting"
      @close="editingMeeting = false"
    />
  </section>
</template>
