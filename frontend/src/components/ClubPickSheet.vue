<script setup lang="ts">
import { ref } from "vue";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  title: string;
  bookTitle: string;
  timezone: string;
  meetingLocal?: string | null;
  note?: string;
  confirmLabel?: string;
  hideNote?: boolean;
}>();

const emit = defineEmits<{
  confirm: [meetingAt: string | null, note: string];
  close: [];
}>();

const meeting = ref(props.meetingLocal ?? "");
const note = ref(props.note ?? "");
</script>

<template>
  <Sheet :title="title" :subtitle="bookTitle" @close="emit('close')">
    <label class="field">
      <span>Meeting <span class="faint">(optional · {{ timezone }})</span></span>
      <input v-model="meeting" type="datetime-local" data-autofocus />
    </label>
    <button v-if="meeting" class="text-btn sm" type="button" style="margin: -6px 0 10px" @click="meeting = ''">
      Clear meeting
    </button>
    <label v-if="!hideNote" class="field">
      <span>Why this book? <span class="faint">(optional)</span></span>
      <textarea v-model="note" rows="3" maxlength="280" placeholder="A line for the club — what drew you to it?" />
      <span class="field-hint">{{ note.length }}/280</span>
    </label>
    <template #foot>
      <button class="btn btn-primary" type="button" @click="emit('confirm', meeting.trim() || null, note.trim())">
        {{ confirmLabel || "Set club pick" }}
      </button>
    </template>
  </Sheet>
</template>
