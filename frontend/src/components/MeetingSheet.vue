<script setup lang="ts">
import { ref } from "vue";
import BottomSheet from "./BottomSheet.vue";

const props = defineProps<{
  title: string;
  bookTitle: string;
  timezone: string;
  meetingLocal?: string | null;
  confirmLabel?: string;
  blurb?: string;
}>();

const emit = defineEmits<{
  confirm: [meetingAt: string | null];
  close: [];
}>();

const meeting = ref(props.meetingLocal ?? "");
</script>

<template>
  <BottomSheet :title="title" @close="emit('close')">
    <p class="muted sheet-book">{{ bookTitle }}</p>
    <p v-if="blurb" class="fine subtle sheet-blurb">{{ blurb }}</p>
    <label class="field">
      <span>Meeting date and time</span>
      <input v-model="meeting" type="datetime-local" />
      <span class="field-hint">Optional · {{ timezone }}</span>
    </label>
    <button
      v-if="meeting"
      class="text-btn clear-btn"
      type="button"
      @click="meeting = ''"
    >
      Clear meeting
    </button>
    <div class="stack">
      <button
        class="btn btn-primary btn-block"
        type="button"
        @click="emit('confirm', meeting.trim() || null)"
      >
        {{ confirmLabel || "Set club pick" }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" @click="emit('close')">
        Cancel
      </button>
    </div>
  </BottomSheet>
</template>

<style scoped>
.sheet-book {
  font-family: var(--serif);
  font-size: var(--text-lg);
  margin-bottom: var(--space-2);
}

.sheet-blurb {
  margin-bottom: var(--space-4);
}

.clear-btn {
  margin: calc(-1 * var(--space-2)) 0 var(--space-3);
}
</style>
