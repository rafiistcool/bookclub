<script setup lang="ts">
import { computed } from "vue";
import { avatarColor, initials } from "../constants";

const props = defineProps<{
  username: string;
  size?: "sm" | "md" | "lg";
  title?: string;
}>();

const color = computed(() => avatarColor(props.username));
const label = computed(() => initials(props.username));
</script>

<template>
  <span
    class="avatar"
    :class="size ?? 'md'"
    :style="{ '--avatar-bg': color }"
    :title="title ?? username"
    :aria-label="username"
    role="img"
  >
    {{ label }}
  </span>
</template>
