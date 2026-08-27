<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { STATUSES, STATUS_LABEL, type FinishNote, type Status } from "../constants";

const props = defineProps<{
  title: string;
  current?: Status | null;
  clubPickLabel?: string;
  nominateLabel?: string;
}>();

const emit = defineEmits<{
  pick: [status: Status, note?: FinishNote];
  clubPick: [];
  nominate: [];
  close: [];
  finish: [status: Extract<Status, "finished" | "did_not_finish">];
}>();

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

function choose(status: Status) {
  if (status === "finished" || status === "did_not_finish") {
    emit("finish", status);
    return;
  }
  emit("pick", status);
}

onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="sheet-backdrop" @click.self="emit('close')">
    <div class="sheet" role="dialog" aria-modal="true" :aria-label="title">
      <h2>{{ title }}</h2>
      <div class="status-list">
        <button
          v-for="status in STATUSES"
          :key="status"
          class="btn"
          :class="{ 'btn-primary': status === (props.current ?? 'want_to_read') }"
          :aria-current="status === props.current ? 'true' : undefined"
          type="button"
          @click="choose(status)"
        >
          {{ STATUS_LABEL[status] }}
        </button>
      </div>
      <button
        v-if="clubPickLabel"
        class="btn btn-ghost"
        type="button"
        style="width: 100%; margin-top: 10px"
        @click="emit('clubPick')"
      >
        {{ clubPickLabel }}
      </button>
      <button
        v-if="nominateLabel"
        class="btn btn-ghost"
        type="button"
        style="width: 100%; margin-top: 10px"
        @click="emit('nominate')"
      >
        {{ nominateLabel }}
      </button>
      <button class="btn btn-ghost" type="button" style="width: 100%; margin-top: 10px" @click="emit('close')">
        Cancel
      </button>
    </div>
  </div>
</template>
