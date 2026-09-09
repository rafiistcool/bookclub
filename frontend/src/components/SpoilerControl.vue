<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { spoilerLabel } from "../diary";

const { t } = useI18n();

/**
 * "Safe up to X%" for a diary entry. A chip shows the current flag; tapping it
 * opens a slider. `null` means no flag: the entry is shown to everyone.
 */
const model = defineModel<number | null>({ required: true });
const open = ref(false);

function onSlide(event: Event) {
  model.value = Number((event.target as HTMLInputElement).value);
}
</script>

<template>
  <span class="spoiler-control">
    <button
      class="chip"
      :class="{ active: model != null }"
      type="button"
      :aria-expanded="open"
      @click="open = !open"
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12Z" />
        <circle cx="12" cy="12" r="2.6" />
        <path v-if="model != null" d="M4 4l16 16" />
      </svg>
      {{ spoilerLabel(model) }}
    </button>
    <span v-if="open" class="spoiler-panel">
      <label class="spoiler-slider">
        <span class="fine">{{ t("diary.readersBlur") }}</span>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          :value="model ?? 0"
          @input="onSlide"
        />
      </label>
      <button
        v-if="model != null"
        class="text-btn"
        type="button"
        @click="
          model = null;
          open = false;
        "
      >
        {{ t("diary.removeFlag") }}
      </button>
    </span>
  </span>
</template>

<style scoped>
.spoiler-control {
  display: grid;
  gap: var(--space-2);
}

.spoiler-control .chip {
  gap: var(--space-1);
  min-height: 30px;
  font-size: var(--text-xs);
}

.spoiler-panel {
  display: grid;
  gap: var(--space-1);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface-2);
}

.spoiler-slider {
  display: grid;
  gap: var(--space-1);
}

.spoiler-slider input {
  width: min(100%, 260px);
  accent-color: var(--accent);
}

.spoiler-panel .text-btn {
  justify-self: start;
}
</style>
