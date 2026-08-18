<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import ShelfBoard from "../components/ShelfBoard.vue";
import StatusSheet from "../components/StatusSheet.vue";
import type { Status } from "../constants";
import { STATUS_LABEL } from "../constants";
import { useToast } from "../stores/toast";
import type { ShelfItem } from "../types";

const items = ref<ShelfItem[]>([]);
const error = ref("");
const loaded = ref(false);
const toast = useToast();
const moving = ref<ShelfItem | null>(null);
const removing = ref<ShelfItem | null>(null);

async function load() {
  try {
    items.value = (await api.myShelf()).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load your shelf";
  } finally {
    loaded.value = true;
  }
}

async function dropped(item: ShelfItem, status: Status, position: number) {
  const previous = items.value.map((row) => ({ ...row }));
  try {
    await api.patchShelf(item.id, { status, position });
    items.value = (await api.myShelf()).items;
    error.value = "";
    return true;
  } catch (err) {
    items.value = previous;
    toast.show(err instanceof ApiError ? err.message : "Could not move that book");
    return false;
  }
}

async function moveTo(status: Status) {
  const item = moving.value;
  moving.value = null;
  if (!item) return;
  if (await dropped(item, status, 0)) {
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
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

onMounted(load);
</script>

<template>
  <section>
    <h1>Your shelf</h1>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <template v-else>
      <p v-if="items.length === 0" class="empty">
        Nothing here yet.
        <RouterLink to="/library">Find a book in the library.</RouterLink>
      </p>
      <ShelfBoard
        :items="items"
        @dropped="dropped"
        @move="moving = $event"
        @remove="removing = $event"
      />
    </template>
    <StatusSheet
      v-if="moving"
      :title="`Move “${moving.book.title}”`"
      :current="moving.status"
      @pick="moveTo"
      @close="moving = null"
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
