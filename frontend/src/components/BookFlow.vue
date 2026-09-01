<script setup lang="ts">
/** Mounted once in the shell. Renders whichever book sheet is open. */
import { computed } from "vue";
import type { FinishNote, Status } from "../constants";
import { useFlow, type Flow } from "../stores/flow";
import { usePick } from "../stores/pick";
import BookActionsSheet from "./BookActionsSheet.vue";
import BookDetailSheet from "./BookDetailSheet.vue";
import ClubPickSheet from "./ClubPickSheet.vue";
import FinishNoteSheet from "./FinishNoteSheet.vue";
import ProgressSheet from "./ProgressSheet.vue";
import QuoteSheet from "./QuoteSheet.vue";
import ScanSheet from "./ScanSheet.vue";
import Sheet from "./Sheet.vue";
import StatusSheet from "./StatusSheet.vue";

const flow = useFlow();
const pick = usePick();
const current = computed(() => flow.current);

type Of<K extends Flow["kind"]> = Extract<Flow, { kind: K }>;

const details = computed(() => (current.value?.kind === "details" ? (current.value as Of<"details">) : null));
const status = computed(() => (current.value?.kind === "status" ? (current.value as Of<"status">) : null));
const finish = computed(() => (current.value?.kind === "finish" ? (current.value as Of<"finish">) : null));
const progress = computed(() => (current.value?.kind === "progress" ? (current.value as Of<"progress">) : null));
const clubPick = computed(() => (current.value?.kind === "clubPick" ? (current.value as Of<"clubPick">) : null));
const actions = computed(() => (current.value?.kind === "actions" ? (current.value as Of<"actions">) : null));
const quote = computed(() => (current.value?.kind === "quote" ? (current.value as Of<"quote">) : null));
const remove = computed(() => (current.value?.kind === "remove" ? (current.value as Of<"remove">) : null));
const scan = computed(() => current.value?.kind === "scan");

const pickMeeting = computed(() =>
  clubPick.value && pick.pick?.book.ol_work_key === clubPick.value.book.ol_work_key ? pick.pick.meeting_local : "",
);
const pickNote = computed(() =>
  clubPick.value && pick.pick?.book.ol_work_key === clubPick.value.book.ol_work_key ? pick.pick.note : "",
);

function onStatus(next: Status) {
  const f = status.value;
  if (f) void flow.chooseStatus(f.book, f.item, next, f.item?.position ?? 0);
}

function onFinish(note: FinishNote) {
  const f = finish.value;
  if (f) void flow.applyStatus(f.book, f.item, f.status, f.position, note);
}

function onProgress(value: number | null) {
  const f = progress.value;
  if (f) void flow.setProgress(f.item, value);
}

function onClubPick(meetingAt: string | null, note: string) {
  const f = clubPick.value;
  if (f) void flow.setClubPick(f.book, meetingAt, note);
}

function onRemove() {
  const f = remove.value;
  if (f) void flow.remove(f.item);
}
</script>

<template>
  <BookDetailSheet v-if="details" :key="details.book.ol_work_key" :book="details.book" @close="flow.close()" />

  <StatusSheet
    v-else-if="status"
    :title="status.title ?? (status.item ? 'Move to…' : 'Add to shelf')"
    :book-title="status.book.title"
    :current="status.item?.status ?? null"
    @pick="onStatus"
    @close="flow.close()"
  />

  <FinishNoteSheet
    v-else-if="finish"
    :book-title="finish.book.title"
    :status="finish.status"
    :rating="finish.item?.rating"
    :take="finish.item?.take"
    :dnf-reason="finish.item?.dnf_reason"
    @confirm="onFinish"
    @close="flow.close()"
  />

  <ProgressSheet
    v-else-if="progress"
    :book-title="progress.item.book.title"
    :progress="progress.item.progress"
    :pages="progress.item.book.pages"
    @confirm="onProgress"
    @close="flow.close()"
  />

  <ClubPickSheet
    v-else-if="clubPick"
    title="Set club pick"
    :book-title="clubPick.book.title"
    :timezone="pick.timezone"
    :meeting-local="pickMeeting"
    :note="pickNote"
    @confirm="onClubPick"
    @close="flow.close()"
  />

  <BookActionsSheet v-else-if="actions" :item="actions.item" @close="flow.close()" />

  <QuoteSheet v-else-if="quote" :book="quote.book" @close="flow.close()" />

  <ScanSheet v-else-if="scan" @close="flow.close()" />

  <Sheet
    v-else-if="remove"
    :title="`Remove “${remove.item.book.title}”?`"
    subtitle="It leaves your shelf. You can undo for a few seconds."
    @close="flow.close()"
  >
    <template #foot>
      <button class="btn btn-danger" type="button" data-autofocus @click="onRemove">Remove</button>
      <button class="btn btn-ghost" type="button" @click="flow.close()">Cancel</button>
    </template>
  </Sheet>
</template>
