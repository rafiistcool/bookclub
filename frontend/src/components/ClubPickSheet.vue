<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const props = defineProps<{
  title: string;
  bookTitle: string;
  timezone: string;
  meetingLocal?: string | null;
  confirmLabel?: string;
}>();

const emit = defineEmits<{
  confirm: [meetingAt: string | null];
  close: [];
}>();

const meeting = ref(props.meetingLocal ?? "");

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="sheet-backdrop" @click.self="emit('close')">
    <div class="sheet" role="dialog" aria-modal="true" :aria-label="title">
      <h2>{{ title }}</h2>
      <p class="muted" style="margin-bottom: 14px">{{ bookTitle }}</p>
      <label class="field">
        <span>Meeting (optional, {{ timezone }})</span>
        <input v-model="meeting" type="datetime-local" />
      </label>
      <button
        v-if="meeting"
        class="text-btn"
        type="button"
        style="margin: -8px 0 12px"
        @click="meeting = ''"
      >
        Clear meeting
      </button>
      <button class="btn btn-primary" type="button" @click="emit('confirm', meeting.trim() || null)">
        {{ confirmLabel || "Set club pick" }}
      </button>
      <button class="btn btn-ghost" type="button" style="width: 100%; margin-top: 10px" @click="emit('close')">
        Cancel
      </button>
    </div>
  </div>
</template>
