<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import ShelfList from "../components/ShelfList.vue";
import Skeleton from "../components/Skeleton.vue";
import NavIcon from "../components/NavIcon.vue";
import type { Status } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useShelf } from "../stores/shelf";
import type { ShelfItem } from "../types";

const shelf = useShelf();
const pickStore = usePick();
const flow = useFlow();

const wide = ref(window.matchMedia("(min-width: 720px)").matches);
const media = window.matchMedia("(min-width: 720px)");
const onMedia = (event: MediaQueryListEvent) => {
  wide.value = event.matches;
};

const clubKey = computed(() => pickStore.pick?.book.ol_work_key ?? null);
const counts = computed(() => ({
  reading: shelf.items.filter((row) => row.status === "currently_reading").length,
  finished: shelf.items.filter((row) => row.status === "finished").length,
  total: shelf.items.length,
}));

async function onDropped(item: ShelfItem, status: Status, position: number) {
  await flow.chooseStatus(toRef(item.book), item, status, position);
}

onMounted(() => {
  media.addEventListener("change", onMedia);
  void shelf.load();
  void pickStore.load();
});

onUnmounted(() => media.removeEventListener("change", onMedia));
</script>

<template>
  <section aria-label="Your shelf">
    <p v-if="shelf.error && !shelf.loaded" class="error">{{ shelf.error }}</p>
    <Skeleton v-else-if="!shelf.loaded" kind="cards" :count="4" />

    <div v-else-if="shelf.items.length === 0" class="empty">
      <p class="kicker">Empty shelf</p>
      <p>Find a book in the <RouterLink to="/library">library</RouterLink>, scan one, or import your Goodreads export in <RouterLink to="/settings">Settings</RouterLink>.</p>
      <div class="actions" style="justify-content: center">
        <RouterLink class="btn btn-primary" to="/library">Browse the library</RouterLink>
        <button class="btn btn-ghost" type="button" @click="flow.open({ kind: 'scan' })">
          <NavIcon name="camera" :size="18" /> Scan
        </button>
      </div>
    </div>

    <template v-else>
      <p class="fine muted" style="margin-bottom: 12px">
        {{ counts.total }} books · {{ counts.reading }} reading · {{ counts.finished }} finished
        <template v-if="wide"> · drag between columns</template>
      </p>
      <ShelfBoard v-if="wide" :items="shelf.items" :club-pick-key="clubKey" @dropped="onDropped" />
      <ShelfList v-else :items="shelf.items" :club-pick-key="clubKey" />
    </template>
  </section>
</template>
