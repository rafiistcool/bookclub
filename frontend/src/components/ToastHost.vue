<script setup lang="ts">
import { useToast } from "../stores/toast";

const toast = useToast();
</script>

<template>
  <div v-if="toast.visible" class="toast" role="status">{{ toast.message }}</div>
</template>
