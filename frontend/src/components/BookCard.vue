<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  bookPath,
  starLabel,
  statusShort,
  type Status,
} from "../constants";
import type { ShelfItem } from "../types";
import BookCover from "./BookCover.vue";

const props = defineProps<{
  item: ShelfItem;
  readonly?: boolean;
  clubPickKey?: string | null;
}>();

const emit = defineEmits<{
  remove: [];
  clubPick: [];
  nominate: [];
}>();

const menuOpen = ref(false);
const root = ref<HTMLElement | null>(null);

function onDocumentPointerDown(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) menuOpen.value = false;
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") menuOpen.value = false;
}

function toggleMenu() {
  menuOpen.value = !menuOpen.value;
  if (menuOpen.value) {
    document.addEventListener("pointerdown", onDocumentPointerDown);
    document.addEventListener("keydown", onDocumentKeydown);
  } else {
    stopListening();
  }
}

function closeMenu() {
  menuOpen.value = false;
  stopListening();
}

function stopListening() {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  document.removeEventListener("keydown", onDocumentKeydown);
}

onUnmounted(stopListening);

const { t } = useI18n();

const isClubPick = () =>
  Boolean(props.clubPickKey) && props.item.book.ol_work_key === props.clubPickKey;
</script>

<template>
  <article ref="root" class="shelf-card">
    <RouterLink class="card-link" :to="bookPath(item.book.ol_work_key)">
      <BookCover
        :title="item.book.title"
        :cover-id="item.book.cover_id"
        :work-key="item.book.ol_work_key"
        :image-url="item.book.cover_url"
        size="sm"
      />
      <span class="card-meta">
        <strong class="card-title">{{ item.book.title }}</strong>
        <span v-if="item.book.authors" class="finer subtle">{{ item.book.authors }}</span>
        <span class="meta-line">
          <span v-if="isClubPick()" class="badge club-pick">{{ t("common.club") }}</span>
          <span class="badge" :class="item.status">{{ statusShort(item.status) }}</span>
          <span v-if="item.progress != null" class="finer subtle nums">
            {{ item.progress }}%
          </span>
          <span v-if="item.rating" class="finer stars">{{ starLabel(item.rating) }}</span>
        </span>
        <span v-if="item.take" class="note finer">{{ item.take }}</span>
        <span v-else-if="item.dnf_reason" class="note finer">{{ item.dnf_reason }}</span>
      </span>
    </RouterLink>

    <div v-if="!readonly" class="card-actions no-drag">
      <button
        class="icon-btn"
        type="button"
        :aria-label="t('card.actions')"
        :aria-expanded="menuOpen"
        @click="toggleMenu"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="5" cy="12" r="1.8" fill="currentColor" />
          <circle cx="12" cy="12" r="1.8" fill="currentColor" />
          <circle cx="19" cy="12" r="1.8" fill="currentColor" />
        </svg>
      </button>
      <div v-if="menuOpen" class="card-menu">
        <RouterLink :to="bookPath(item.book.ol_work_key)">{{ t("card.openBook") }}</RouterLink>
        <button type="button" @click="closeMenu(); emit('clubPick')">
          {{ t("card.setPick") }}
        </button>
        <button type="button" @click="closeMenu(); emit('nominate')">
          {{ t("card.nominate") }}
        </button>
        <button type="button" @click="closeMenu(); emit('remove')">{{ t("common.remove") }}</button>
      </div>
    </div>
  </article>
</template>

<style scoped>
.shelf-card {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: var(--space-1);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--space-2);
  touch-action: manipulation;
}

.card-link {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: var(--space-3);
  color: inherit;
  text-decoration: none;
}

.card-meta {
  min-width: 0;
  display: grid;
  gap: 2px;
  align-content: start;
}

.card-title {
  font-family: var(--serif);
  font-size: var(--text-md);
  line-height: var(--leading-snug);
}

.card-actions {
  position: relative;
  flex: 0 0 auto;
}

.card-menu {
  position: absolute;
  right: 0;
  top: calc(var(--tap) - 4px);
  min-width: 190px;
  padding: var(--space-1);
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: var(--elev-float);
  z-index: 30;
}

.card-menu a,
.card-menu button {
  display: block;
  width: 100%;
  text-align: left;
  border: 0;
  background: transparent;
  padding: var(--space-3);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-weight: 600;
  font-size: var(--text-sm);
  text-decoration: none;
}

.card-menu a:hover,
.card-menu button:hover {
  background: var(--surface-2);
}
</style>
