<script setup lang="ts">
import { useToast } from "../stores/toast";

const toast = useToast();
</script>

<template>
  <!-- Always mounted: the live region has to exist before a message lands in
       it to be announced, and a leaving toast needs its host to animate in. -->
  <div class="toast-host" role="status" aria-live="polite">
    <TransitionGroup name="toast">
      <div v-for="row in toast.items" :key="row.id" class="toast">
        <span>{{ row.message }}</span>
        <button v-if="row.action" type="button" @click="toast.run(row.id)">
          {{ row.action.label }}
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
/* A toast rises from the tab bar and settles; it leaves faster, dropping
   back. The others close the gap on the move transition. */
.toast-enter-active {
  transition:
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-slow) var(--ease-out);
}

.toast-leave-active {
  transition:
    opacity var(--dur-fast) var(--ease-in),
    transform var(--dur-fast) var(--ease-in);
}

.toast-move {
  transition: transform var(--dur-base) var(--ease-out);
}

.toast-enter-from {
  opacity: 0;
  transform: translateY(16px) scale(0.96);
}

.toast-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
</style>
