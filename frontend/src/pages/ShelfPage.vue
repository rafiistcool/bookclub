<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import ClubPickBanner from "../components/ClubPickBanner.vue";
import ClubPickSheet from "../components/ClubPickSheet.vue";
import FinishNoteSheet from "../components/FinishNoteSheet.vue";
import ProgressSheet from "../components/ProgressSheet.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import StatusSheet from "../components/StatusSheet.vue";
import type { FinishNote, Status } from "../constants";
import { STATUS_LABEL } from "../constants";
import { useToast } from "../stores/toast";
import type { ClubPick, GoodreadsImport, ShelfItem } from "../types";

const router = useRouter();
const items = ref<ShelfItem[]>([]);
const error = ref("");
const loaded = ref(false);
const toast = useToast();
const moving = ref<ShelfItem | null>(null);
const finishing = ref<{
  item: ShelfItem;
  status: "finished" | "did_not_finish";
  position: number;
} | null>(null);
const removing = ref<ShelfItem | null>(null);
const progressing = ref<ShelfItem | null>(null);
const clubPick = ref<ClubPick | null>(null);
const clubTimezone = ref("UTC");
const settingPick = ref<ShelfItem | null>(null);
const importFile = ref<File | null>(null);
const importing = ref(false);
const importResult = ref<GoodreadsImport | null>(null);

async function loadClubPick() {
  try {
    const current = await api.clubPick();
    clubPick.value = current.pick;
    clubTimezone.value = current.timezone;
  } catch {
    clubPick.value = null;
  }
}

async function load() {
  try {
    items.value = (await api.myShelf()).items;
    error.value = "";
    await loadClubPick();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load your shelf";
  } finally {
    loaded.value = true;
  }
}

async function addClubPickToShelf() {
  const current = clubPick.value;
  if (!current) return;
  const existing = items.value.find((row) => row.book.ol_work_key === current.book.ol_work_key);
  if (existing) {
    moving.value = existing;
    return;
  }
  try {
    const created = await api.addToShelf({
      ol_work_key: current.book.ol_work_key,
      title: current.book.title,
      authors: current.book.authors,
      cover_id: current.book.cover_id,
      year: current.book.year,
      status: "currently_reading",
    });
    items.value = (await api.myShelf()).items;
    current.on_shelf = created.status;
    current.shelf_id = created.id;
    toast.show(`Added to ${STATUS_LABEL[created.status]}`);
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      items.value = (await api.myShelf()).items;
      moving.value = err.item;
      return;
    }
    toast.show(err instanceof ApiError ? err.message : "Could not add that book");
  }
}

function moveClubPick() {
  const current = clubPick.value;
  if (!current) return;
  const existing = items.value.find((row) => row.book.ol_work_key === current.book.ol_work_key);
  if (existing) moving.value = existing;
}

function startClubPick(item: ShelfItem) {
  settingPick.value = item;
}

async function nominate(item: ShelfItem) {
  try {
    await api.nominate({
      ol_work_key: item.book.ol_work_key,
      title: item.book.title,
      authors: item.book.authors,
      cover_id: item.book.cover_id,
      year: item.book.year,
    });
    toast.show("Nominated for next up");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
  }
}

async function confirmClubPick(meetingAt: string | null) {
  const item = settingPick.value;
  settingPick.value = null;
  if (!item) return;
  try {
    clubPick.value = await api.setClubPick({
      ol_work_key: item.book.ol_work_key,
      title: item.book.title,
      authors: item.book.authors,
      cover_id: item.book.cover_id,
      year: item.book.year,
      meeting_at: meetingAt,
    });
    toast.show("Set as the club pick");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the club pick");
  }
}

async function dropped(item: ShelfItem, status: Status, position: number, note: FinishNote = {}) {
  const previous = items.value.map((row) => ({ ...row }));
  try {
    await api.patchShelf(item.id, { status, position, ...note });
    items.value = (await api.myShelf()).items;
    error.value = "";
    return true;
  } catch (err) {
    items.value = previous;
    toast.show(err instanceof ApiError ? err.message : "Could not move that book");
    return false;
  }
}

function requestFinish(item: ShelfItem, status: "finished" | "did_not_finish", position: number) {
  finishing.value = { item, status, position };
}

function onDropped(item: ShelfItem, status: Status, position: number) {
  if (status === "finished" || status === "did_not_finish") {
    requestFinish(item, status, position);
    return;
  }
  void applyMove(item, status, position);
}

async function applyMove(item: ShelfItem, status: Status, position: number, note: FinishNote = {}) {
  if (await dropped(item, status, position, note)) {
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
  }
}

async function moveTo(status: Status) {
  const item = moving.value;
  moving.value = null;
  if (!item) return;
  if (status === "finished" || status === "did_not_finish") {
    requestFinish(item, status, 0);
    return;
  }
  await applyMove(item, status, 0);
}

async function saveFinish(note: FinishNote) {
  const pending = finishing.value;
  finishing.value = null;
  if (!pending) return;
  await applyMove(pending.item, pending.status, pending.position, note);
}

async function cancelFinish() {
  finishing.value = null;
  items.value = (await api.myShelf()).items;
}

async function saveProgress(progress: number | null) {
  const item = progressing.value;
  progressing.value = null;
  if (!item) return;
  try {
    await api.patchShelf(item.id, { progress });
    items.value = (await api.myShelf()).items;
    toast.show(progress == null ? "Progress cleared" : `Progress: ${progress}%`);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save progress");
  }
}

async function confirmRemove() {
  const item = removing.value;
  removing.value = null;
  if (!item) return;
  try {
    await api.removeFromShelf(item.id);
    items.value = items.value.filter((row) => row.id !== item.id);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not remove that book");
  }
}

function onImportFile(event: Event) {
  const input = event.target as HTMLInputElement;
  importFile.value = input.files?.[0] ?? null;
  importResult.value = null;
}

async function importCsv() {
  const file = importFile.value;
  if (!file || importing.value) return;
  importing.value = true;
  try {
    const result = await api.importGoodreads(file);
    importResult.value = result;
    items.value = (await api.myShelf()).items;
    if (result.imported) {
      toast.show(
        result.skipped
          ? `Imported ${result.imported}. Skipped ${result.skipped}.`
          : `Imported ${result.imported}`,
      );
    } else {
      toast.show(result.skipped ? `Skipped ${result.skipped}` : "Nothing to import");
    }
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not import that file");
  } finally {
    importing.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section>
    <h1>Your shelf</h1>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <template v-else>
      <ClubPickBanner
        :pick="clubPick"
        compact
        empty-hint="Set a club pick from a book on your shelf."
        @add="addClubPickToShelf"
        @move="moveClubPick"
        @open="router.push('/')"
      />
      <p v-if="items.length === 0" class="empty">
        Nothing here yet.
        <RouterLink to="/library">Find a book in the library</RouterLink>
        or import a Goodreads export below.
      </p>
      <ShelfBoard
        v-if="items.length"
        :items="items"
        :club-pick-key="clubPick?.book.ol_work_key"
        @dropped="onDropped"
        @move="moving = $event"
        @remove="removing = $event"
        @club-pick="startClubPick"
        @nominate="nominate"
        @progress="progressing = $event"
      />
      <section class="import-block">
        <h2>Import from Goodreads</h2>
        <p class="muted fine" style="margin-bottom: 12px">
          Upload a Goodreads library export CSV. Exclusive shelves map to Want to read,
          Reading, and Finished. Rows we cannot match are skipped.
        </p>
        <label class="import-file">
          <input type="file" accept=".csv,text/csv" @change="onImportFile" />
          <span>{{ importFile ? importFile.name : "Choose CSV" }}</span>
        </label>
        <button
          class="btn btn-ghost"
          type="button"
          :disabled="!importFile || importing"
          @click="importCsv"
        >
          {{ importing ? "Importing…" : "Import CSV" }}
        </button>
        <p v-if="importResult" class="muted fine" style="margin-top: 12px">
          Imported {{ importResult.imported }}. Skipped {{ importResult.skipped }}.
        </p>
        <ul v-if="importResult?.skips.length" class="import-skips">
          <li v-for="(skip, index) in importResult.skips" :key="index">
            {{ skip.title || "Untitled" }} — {{ skip.reason }}
          </li>
        </ul>
      </section>
    </template>
    <ClubPickSheet
      v-if="settingPick"
      title="Set club pick"
      :book-title="settingPick.book.title"
      :timezone="clubTimezone"
      :meeting-local="clubPick?.book.ol_work_key === settingPick.book.ol_work_key ? clubPick.meeting_local : ''"
      @confirm="confirmClubPick"
      @close="settingPick = null"
    />
    <StatusSheet
      v-if="moving"
      :title="`Move “${moving.book.title}”`"
      :current="moving.status"
      @pick="moveTo"
      @finish="(status) => { if (moving) requestFinish(moving, status, 0); moving = null }"
      @close="moving = null"
    />
    <ProgressSheet
      v-if="progressing"
      title="Reading progress"
      :book-title="progressing.book.title"
      :progress="progressing.progress"
      @confirm="saveProgress"
      @close="progressing = null"
    />
    <FinishNoteSheet
      v-if="finishing"
      :title="finishing.status === 'finished' ? 'Finished' : 'Did not finish'"
      :book-title="finishing.item.book.title"
      :status="finishing.status"
      :rating="finishing.item.rating"
      :take="finishing.item.take"
      :dnf-reason="finishing.item.dnf_reason"
      @confirm="saveFinish"
      @close="cancelFinish"
    />
    <div v-if="removing" class="sheet-backdrop" @click.self="removing = null">
      <div class="sheet" role="dialog" aria-modal="true">
        <h2>Remove “{{ removing.book.title }}”?</h2>
        <p class="muted" style="margin-bottom: 14px">It leaves your shelf. You can add it again later.</p>
        <button class="btn btn-danger" type="button" style="width: 100%" @click="confirmRemove">
          Remove
        </button>
        <button class="btn btn-ghost" type="button" style="width: 100%; margin-top: 8px" @click="removing = null">
          Cancel
        </button>
      </div>
    </div>
  </section>
</template>
