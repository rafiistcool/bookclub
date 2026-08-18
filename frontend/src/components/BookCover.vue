<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { coverUrl, monogram } from "../constants";

const props = defineProps<{
  title: string;
  coverId?: number | null;
  size?: "S" | "M" | "L";
  tiny?: boolean;
  fluid?: boolean;
}>();

const broken = ref(false);
const src = computed(() => coverUrl(props.coverId, props.size ?? "L"));

watch(
  () => props.coverId,
  () => {
    broken.value = false;
  },
);
</script>

<template>
  <div class="cover" :class="{ tiny, fluid }">
    <img
      v-if="src && !broken"
      :src="src"
      :alt="title"
      @error="broken = true"
    />
    <span v-else class="monogram" aria-hidden="true">{{ monogram(title) }}</span>
  </div>
</template>
