<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { coverUrl, monogram } from "../constants";

const props = defineProps<{
  title: string;
  coverId?: number | null;
  /** Layout width. Image resolution is picked to match. */
  size?: "xs" | "sm" | "md" | "lg" | "fluid";
  eager?: boolean;
}>();

const broken = ref(false);
const resolution = computed(() => {
  switch (props.size) {
    case "xs":
    case "sm":
      return "S" as const;
    case "md":
      return "M" as const;
    default:
      return "L" as const;
  }
});
const src = computed(() => coverUrl(props.coverId, resolution.value));

watch(
  () => props.coverId,
  () => {
    broken.value = false;
  },
);
</script>

<template>
  <div class="cover" :class="size ?? 'md'">
    <img
      v-if="src && !broken"
      :src="src"
      :alt="title"
      :loading="eager ? 'eager' : 'lazy'"
      decoding="async"
      @error="broken = true"
    />
    <span v-else class="monogram" aria-hidden="true">{{ monogram(title) }}</span>
  </div>
</template>
