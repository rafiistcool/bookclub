<script setup lang="ts">
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import { tp } from "../i18n";
import Avatar from "../components/Avatar.vue";
import BottomSheet from "../components/BottomSheet.vue";
import PhysicalShelf from "../components/PhysicalShelf.vue";
import ShelfBookInfo from "../components/ShelfBookInfo.vue";
import ShelfViewSwitch from "../components/ShelfViewSwitch.vue";
import { reconcileLayout, type PhysicalShelfLayout, type ShelfObjectId } from "../physicalShelf";
import type { ShelfItem } from "../types";

const { t } = useI18n();
const route = useRoute();
const username = ref("");
const avatarUrl = ref<string | null>(null);
const items = ref<ShelfItem[]>([]);
const layout = ref<PhysicalShelfLayout>({ rows: [[], [], []], material: "oak", spines: {} });
const loaded = ref(false);
const error = ref("");
const selectedId = ref<number | null>(null);
const selected = computed(() => items.value.find(item => item.id === selectedId.value) ?? null);

async function load() {
  loaded.value = false;
  selectedId.value = null;
  username.value = String(route.params.username || "");
  try {
    const [shelf, saved] = await Promise.all([api.friendShelf(username.value), api.physicalShelfLayout(username.value)]);
    username.value = shelf.user.username;
    avatarUrl.value = shelf.user.avatar_url ?? null;
    items.value = shelf.items;
    // A member who never arranged their bookshelf still gets one, filled in shelf order.
    layout.value = reconcileLayout(saved, shelf.items);
    error.value = "";
  } catch (err) {
    items.value = [];
    error.value = err instanceof ApiError ? err.message : t("memberShelf.loadFailed");
  } finally {
    loaded.value = true;
  }
}

function select(id: ShelfObjectId) {
  if (typeof id === "number") selectedId.value = id;
}

onMounted(load);
watch(() => route.params.username, load);
</script>

<template>
  <section class="member-physical-shelf">
    <p class="fine back-link"><RouterLink to="/club">{{ t("memberShelf.back") }}</RouterLink></p>
    <div class="page-head member-head">
      <Avatar :username="username" :src="avatarUrl" size="lg" />
      <div>
        <h1>{{ t("physicalShelf.memberTitle", { name: username }) }}</h1>
        <p v-if="loaded && !error" class="lede nums">{{ tp("shelf.books", items.length) }}</p>
      </div>
    </div>
    <div class="member-shelf-switch"><ShelfViewSwitch active="physical" :base="`/club/${username}`" /></div>

    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <PhysicalShelf v-else-if="!loaded" :layout="layout" :items="[]" loading />
    <div v-else-if="items.length === 0" class="empty"><p>{{ t("physicalShelf.memberEmpty", { name: username }) }}</p></div>
    <PhysicalShelf v-else :layout="layout" :items="items" readonly @select="select" />

    <BottomSheet v-if="selected" :title="t('physicalShelf.memberBookSheet', { name: username })" @close="selectedId = null">
      <ShelfBookInfo :item="selected">
        <button type="button" class="icon-btn close-spine-sheet" :aria-label="t('physicalShelf.close')" @click="selectedId = null"><XMarkIcon aria-hidden="true" /></button>
      </ShelfBookInfo>
      <RouterLink class="btn btn-primary btn-block open-book" :to="`/book/${selected.book.ol_work_key.replace('/works/', '')}`">{{ t("physicalShelf.openBook") }}</RouterLink>
    </BottomSheet>
  </section>
</template>

<style scoped>
.member-physical-shelf { max-width: 940px; margin: 0 auto; }
.back-link { margin-bottom: var(--space-4); }
.member-head { display: flex; align-items: center; gap: var(--space-4); }
.member-shelf-switch { margin-bottom: var(--space-3); }
.close-spine-sheet { align-self: flex-start; }
.close-spine-sheet svg { width: 20px; height: 20px; }
.open-book { margin-top: 22px; }
</style>
