<script setup lang="ts">
import { computed } from "vue";
import { clamp } from "../constants";

const props = defineProps<{
  value: number | null | undefined;
  label?: string;
  thin?: boolean;
  hideValue?: boolean;
}>();

const pct = computed(() => clamp(Math.round(props.value ?? 0), 0, 100));
</script>

<template>
  <div class="progress-row">
    <div
      class="progress"
      :class="{ thin }"
      role="progressbar"
      :aria-valuenow="pct"
      aria-valuemin="0"
      aria-valuemax="100"
      :aria-label="label ?? 'Reading progress'"
    >
      <span :style="{ '--value': `${pct}%` }" />
    </div>
    <span v-if="!hideValue">{{ pct }}%</span>
  </div>
</template>
