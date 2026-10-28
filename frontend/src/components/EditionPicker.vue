<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { EditionOut } from "../api/generated";
import { editionLine } from "../editions";
import BottomSheet from "./BottomSheet.vue";

/**
 * "Your edition" on the book page: which edition of the work the reader owns
 * or reads (`ShelfPatchIn.edition_id`). The shelf and the physical shelf then
 * use its pages and cover. Choosing emits the id, or null for no particular one.
 */
const props = defineProps<{ workId: number; editionId: number | null; disabled?: boolean }>();
const emit = defineEmits<{ choose: [editionId: number | null] }>();

const { t } = useI18n();
const editions = ref<EditionOut[] | null>(null);
const error = ref("");
const open = ref(false);

async function load() {
  if (editions.value) return;
  try {
    editions.value = (await api.editions(props.workId)).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("editions.loadFailed");
  }
}

const current = computed(() => editions.value?.find((row) => row.id === props.editionId) ?? null);

function show() {
  open.value = true;
  void load();
}

function choose(editionId: number | null) {
  open.value = false;
  if (editionId !== props.editionId) emit("choose", editionId);
}

onMounted(() => {
  // The label needs the list; without a chosen edition it waits for the sheet.
  if (props.editionId != null) void load();
});
watch(
  () => props.workId,
  () => {
    editions.value = null;
    if (props.editionId != null) void load();
  },
);
</script>

<template>
  <div class="edition-picker">
    <p class="fine edition-current">
      <span class="subtle">{{ t("editions.yours") }}</span>
      <span>{{ editionId == null ? t("editions.any") : current ? editionLine(current) : "…" }}</span>
      <button class="text-btn" type="button" :disabled="disabled" @click="show">
        {{ t("editions.choose") }}
      </button>
    </p>
    <BottomSheet v-if="open" :title="t('editions.title')" @close="open = false">
      <p class="fine muted">{{ t("editions.blurb") }}</p>
      <p v-if="error" class="error">{{ error }}</p>
      <div v-else-if="!editions" class="skeleton editions-skeleton" aria-hidden="true" />
      <ul v-else class="edition-list">
        <li>
          <button class="edition-option" type="button" :aria-pressed="editionId == null" @click="choose(null)">
            {{ t("editions.any") }}
          </button>
        </li>
        <li v-for="edition in editions" :key="edition.id">
          <button
            class="edition-option"
            type="button"
            :aria-pressed="edition.id === editionId"
            @click="choose(edition.id)"
          >
            <strong v-if="edition.title">{{ edition.title }}</strong>
            <span>{{ editionLine(edition) }}</span>
          </button>
        </li>
      </ul>
    </BottomSheet>
  </div>
</template>

<style scoped>
.edition-current {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-2);
  margin: var(--space-2) 0 0;
}

.editions-skeleton {
  height: 120px;
}

.edition-list {
  list-style: none;
  margin: var(--space-3) 0 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.edition-option {
  display: grid;
  gap: 2px;
  width: 100%;
  min-height: var(--tap);
  padding: var(--space-2) var(--space-3);
  text-align: left;
  font: inherit;
  font-size: var(--text-sm);
  color: inherit;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
}

.edition-option[aria-pressed="true"] {
  border-color: var(--accent);
  box-shadow: inset 0 0 0 1px var(--accent);
}
</style>
