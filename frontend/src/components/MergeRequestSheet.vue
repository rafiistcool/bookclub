<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookIdFromInput } from "../constants";
import type { CoverBook } from "../covers";
import { useToast } from "../stores/toast";
import BookCover from "./BookCover.vue";
import BottomSheet from "./BottomSheet.vue";

/**
 * "These are the same book": the catalog has this work twice. The reader finds
 * the other one (a search, or its link pasted) and an admin decides; this
 * work is the one that would be merged into the other.
 */
const props = defineProps<{ workId: number; title: string }>();
const emit = defineEmits<{ close: [] }>();

type Candidate = CoverBook & { id: number; authors: string; year: number | null };

const { t } = useI18n();
const toast = useToast();
const query = ref("");
const hits = ref<Candidate[] | null>(null);
const chosen = ref<Candidate | null>(null);
const note = ref("");
const busy = ref(false);
const error = ref("");

async function find() {
  const text = query.value.trim();
  if (!text || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    const id = bookIdFromInput(text);
    if (id != null) {
      const book = await api.book(id);
      hits.value = [book];
    } else {
      const page = await api.search({ q: text, sort: "relevance", limit: 8 });
      hits.value = page.items;
    }
    hits.value = hits.value.filter((hit) => hit.id !== props.workId);
  } catch (err) {
    hits.value = null;
    error.value = err instanceof ApiError ? err.message : t("merge.findFailed");
  } finally {
    busy.value = false;
  }
}

async function send() {
  if (!chosen.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await api.requestMerge(props.workId, chosen.value.id, note.value.trim());
    toast.show(t("merge.sent"));
    emit("close");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("merge.failed");
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <BottomSheet :title="t('merge.title')" @close="emit('close')">
    <p class="fine muted">{{ t("merge.blurb", { title }) }}</p>
    <form v-if="!chosen" class="stack merge-find" @submit.prevent="find">
      <label class="field">
        <span>{{ t("merge.other") }}</span>
        <input v-model="query" type="search" name="merge-query" :placeholder="t('merge.otherPlaceholder')" :disabled="busy" />
      </label>
      <button class="btn btn-ghost btn-sm" type="submit" :disabled="busy || !query.trim()">{{ t("merge.find") }}</button>
      <p v-if="error" class="error">{{ error }}</p>
      <p v-else-if="hits && !hits.length" class="fine subtle">{{ t("merge.noHits") }}</p>
      <ul v-else-if="hits" class="merge-hits">
        <li v-for="hit in hits" :key="hit.id">
          <button class="merge-hit" type="button" @click="chosen = hit">
            <BookCover :book="hit" size="xs" />
            <span>
              <strong>{{ hit.title }}</strong>
              <span class="fine subtle">{{ [hit.authors, hit.year].filter(Boolean).join(" · ") }}</span>
            </span>
          </button>
        </li>
      </ul>
    </form>
    <form v-else class="stack" @submit.prevent="send">
      <p class="merge-chosen">
        {{ t("merge.same", { title, other: chosen.title }) }}
        <button class="text-btn" type="button" :disabled="busy" @click="chosen = null">{{ t("merge.change") }}</button>
      </p>
      <label class="field">
        <span>{{ t("merge.note") }}</span>
        <textarea v-model="note" rows="3" maxlength="2000" :placeholder="t('common.optional')" :disabled="busy" />
      </label>
      <p v-if="error" class="error">{{ error }}</p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy">{{ t("merge.send") }}</button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.merge-find {
  margin-top: var(--space-3);
}

.merge-hits {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.merge-hit {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  padding: var(--space-2);
  text-align: left;
  font: inherit;
  color: inherit;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
}

.merge-hit > span {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.merge-chosen {
  margin-top: var(--space-3);
}
</style>
