<script setup lang="ts">
import { useToast } from "../stores/toast";

const toast = useToast();
</script>

<template>
  <div v-if="toast.items.length" class="toast-host">
    <TransitionGroup name="toast">
      <div v-for="row in toast.items" :key="row.id" class="toast" role="status">
        <span>{{ row.message }}</span>
        <button v-if="row.action" type="button" @click="toast.run(row.id)">
          {{ row.action.label }}
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-enter-active,
.toast-leave-active {
  transition:
    opacity var(--dur-base) var(--ease),
    transform var(--dur-base) var(--ease);
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>
