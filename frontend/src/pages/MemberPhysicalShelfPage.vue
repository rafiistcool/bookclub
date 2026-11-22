<script setup lang="ts">
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import { tp } from "../i18n";
import Avatar from "../components/Avatar.vue";
import BackLink from "../components/BackLink.vue";
import BottomSheet from "../components/BottomSheet.vue";
import PhysicalShelf from "../components/PhysicalShelf.vue";
import ReportSheet, { type ReportTarget } from "../components/ReportSheet.vue";
import ShelfBookInfo from "../components/ShelfBookInfo.vue";
import ShelfViewSwitch from "../components/ShelfViewSwitch.vue";
import { bookPath } from "../constants";
import { reconcileLayout, type PhysicalShelfLayout, type ShelfObjectId } from "../physicalShelf";
import { useSession } from "../stores/session";
import type { ShelfItem } from "../types";

const { t } = useI18n();
const route = useRoute();
const session = useSession();
const username = ref("");
const avatarUrl = ref<string | null>(null);
const items = ref<ShelfItem[]>([]);
const layout = ref<PhysicalShelfLayout>({ rows: [[], [], []], material: "oak", spines: {} });
const loaded = ref(false);
const error = ref("");
const selectedId = ref<number | null>(null);
const selected = computed(() => items.value.find(item => item.id === selectedId.value) ?? null);
/** The take being reported (#55, Q11 option C); the book's sheet closes for it. */
const reporting = ref<ReportTarget | null>(null);

function reportTake(item: ShelfItem) {
  selectedId.value = null;
  reporting.value = { kind: "take", workId: item.book.id, author: username.value };
}

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
    <BackLink :label="t('memberShelf.back')" fallback="/" />
    <div class="page-head member-head">
      <Avatar :username="username" :src="avatarUrl" size="lg" />
      <div>
        <h1>{{ t("physicalShelf.memberTitle", { name: username }) }}</h1>
        <p v-if="loaded && !error" class="lede nums">{{ tp("shelf.books", items.length) }}</p>
      </div>
    </div>
    <div class="member-shelf-switch"><ShelfViewSwitch active="physical" :base="`/u/${username}`" /></div>

    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <PhysicalShelf v-else-if="!loaded" :layout="layout" :items="[]" loading />
    <div v-else-if="items.length === 0" class="empty"><p>{{ t("physicalShelf.memberEmpty", { name: username }) }}</p></div>
    <PhysicalShelf v-else :layout="layout" :items="items" readonly @select="select" />

    <BottomSheet v-if="selected" :title="t('physicalShelf.memberBookSheet', { name: username })" @close="selectedId = null">
      <ShelfBookInfo :item="selected">
        <button type="button" class="icon-btn close-spine-sheet" :aria-label="t('physicalShelf.close')" @click="selectedId = null"><XMarkIcon aria-hidden="true" /></button>
      </ShelfBookInfo>
      <button
        v-if="selected.take && username !== session.user?.username"
        class="text-btn quiet finer take-report"
        type="button"
        @click="reportTake(selected)"
      >
        {{ t("report.action.take") }}
      </button>
      <RouterLink class="btn btn-primary btn-block open-book" :to="bookPath(selected.book)">{{ t("physicalShelf.openBook") }}</RouterLink>
    </BottomSheet>
    <ReportSheet v-if="reporting" :target="reporting" @close="reporting = null" />
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
.take-report { margin-top: var(--space-2); }
</style>
