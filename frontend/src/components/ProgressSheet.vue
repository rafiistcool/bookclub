<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const props = defineProps<{
  title: string;
  bookTitle: string;
  progress?: number | null;
}>();

const emit = defineEmits<{
  confirm: [progress: number | null];
  close: [];
}>();

const draft = ref(props.progress == null ? "" : String(props.progress));

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

function parsed(): number | null {
  const text = draft.value.trim();
  if (!text) return null;
  const value = Number(text);
  if (!Number.isInteger(value) || value < 0 || value > 100) return null;
  return value;
}

function save() {
  const value = parsed();
  if (draft.value.trim() && value === null) return;
  emit("confirm", value);
}

onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="sheet-backdrop" @click.self="emit('close')">
    <div class="sheet" role="dialog" aria-modal="true" :aria-label="title">
      <h2>{{ title }}</h2>
      <p class="muted" style="margin-bottom: 14px">{{ bookTitle }}</p>
      <label class="field">
        <span>Progress (optional, 0–100)</span>
        <input
          v-model="draft"
          type="number"
          min="0"
          max="100"
          inputmode="numeric"
          placeholder="e.g. 40"
        />
      </label>
      <button
        v-if="draft"
        class="text-btn"
        type="button"
        style="margin: -8px 0 12px"
        @click="draft = ''"
      >
        Clear
      </button>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="Boolean(draft.trim()) && parsed() === null"
        @click="save"
      >
        Save
      </button>
      <button class="btn btn-ghost" type="button" style="width: 100%; margin-top: 10px" @click="emit('close')">
        Cancel
      </button>
    </div>
  </div>
</template>
