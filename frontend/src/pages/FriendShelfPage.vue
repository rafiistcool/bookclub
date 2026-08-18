<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import ShelfBoard from "../components/ShelfBoard.vue";
import type { ShelfItem } from "../types";

const route = useRoute();
const username = ref("");
const items = ref<ShelfItem[]>([]);
const error = ref("");
const loaded = ref(false);

async function load() {
  loaded.value = false;
  username.value = String(route.params.username || "");
  try {
    const shelf = await api.friendShelf(username.value);
    username.value = shelf.user.username;
    items.value = shelf.items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load that shelf";
  } finally {
    loaded.value = true;
  }
}

onMounted(load);
watch(() => route.params.username, load);
</script>

<template>
  <section>
    <p class="fine"><RouterLink to="/friends">← Friends</RouterLink></p>
    <h1 style="margin-top: 8px">{{ username }}’s shelf</h1>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <div v-else-if="items.length === 0" class="empty">Nothing on this shelf yet.</div>
    <ShelfBoard v-else :items="items" readonly />
  </section>
</template>
