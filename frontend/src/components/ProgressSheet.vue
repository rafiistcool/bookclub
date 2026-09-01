<script setup lang="ts">
import { computed, ref } from "vue";
import { clamp } from "../constants";
import ProgressBar from "./ProgressBar.vue";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  bookTitle: string;
  progress?: number | null;
  pages?: number | null;
}>();

const emit = defineEmits<{
  confirm: [progress: number | null];
  close: [];
}>();

const value = ref<number>(props.progress ?? 0);
const pageDraft = ref("");

const pageEstimate = computed(() =>
  props.pages ? Math.round((value.value / 100) * props.pages) : null,
);

function fromPage() {
  if (!props.pages) return;
  const page = Number(pageDraft.value);
  if (!Number.isFinite(page)) return;
  value.value = clamp(Math.round((page / props.pages) * 100), 0, 100);
}

const QUICK = [10, 25, 50, 75, 90];
</script>

<template>
  <Sheet title="Reading progress" :subtitle="bookTitle" @close="emit('close')">
    <div class="field">
      <span class="field-label">How far along are you?</span>
      <ProgressBar :value="value" />
      <input
        v-model.number="value"
        type="range"
        min="0"
        max="100"
        step="1"
        aria-label="Progress percent"
        style="width: 100%; accent-color: var(--accent)"
        data-autofocus
      />
    </div>
    <div class="chip-row" role="group" aria-label="Quick set">
      <button
        v-for="q in QUICK"
        :key="q"
        class="chip"
        type="button"
        :class="{ active: value === q }"
        @click="value = q"
      >
        {{ q }}%
      </button>
    </div>
    <label v-if="pages" class="field" style="margin-top: 8px">
      <span>…or the page you’re on (of {{ pages }})</span>
      <input
        v-model="pageDraft"
        type="number"
        inputmode="numeric"
        min="0"
        :max="pages"
        :placeholder="pageEstimate ? `≈ page ${pageEstimate}` : 'e.g. 120'"
        @input="fromPage"
      />
    </label>
    <template #foot>
      <button class="btn btn-primary" type="button" @click="emit('confirm', value)">Save</button>
      <button v-if="progress != null" class="btn btn-ghost" type="button" @click="emit('confirm', null)">
        Clear progress
      </button>
    </template>
  </Sheet>
</template>
