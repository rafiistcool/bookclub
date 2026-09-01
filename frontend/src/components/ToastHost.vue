<script setup lang="ts">
import { useToast } from "../stores/toast";

const toast = useToast();
</script>

<template>
  <div v-if="toast.visible" class="toast" role="status" aria-live="polite">
    <span>{{ toast.message }}</span>
    <button v-if="toast.action" type="button" @click="toast.act()">{{ toast.action.label }}</button>
  </div>
</template>
