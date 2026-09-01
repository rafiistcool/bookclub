<script setup lang="ts">
import { computed } from "vue";
import { STATUSES, STATUS_LABEL, STATUS_NEXT, type Status } from "../constants";
import NavIcon from "./NavIcon.vue";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  title: string;
  bookTitle?: string;
  current?: Status | null;
}>();

const emit = defineEmits<{
  pick: [status: Status];
  close: [];
}>();

// Current status is marked, never highlighted as the action. The likely next
// step is the primary button; for a new book that is "Want to read".
const primary = computed<Status>(() => (props.current ? STATUS_NEXT[props.current] : "want_to_read"));

const HINT: Record<Status, string> = {
  want_to_read: "Save it for later",
  currently_reading: "Track progress",
  finished: "Rate and leave a take",
  did_not_finish: "Say why, optionally",
};
</script>

<template>
  <Sheet :title="title" :subtitle="bookTitle" @close="emit('close')">
    <div class="status-list">
      <button
        v-for="status in STATUSES"
        :key="status"
        class="btn status-option"
        :class="{ 'btn-primary': status === primary && status !== current, 'btn-ghost': status === current }"
        type="button"
        :aria-current="status === current ? 'true' : undefined"
        :data-autofocus="status === primary ? true : undefined"
        @click="emit('pick', status)"
      >
        <span>{{ STATUS_LABEL[status] }}</span>
        <span v-if="status === current" class="hint" style="display: inline-flex; align-items: center; gap: 4px">
          <NavIcon name="check" :size="16" /> Current
        </span>
        <span v-else class="hint">{{ HINT[status] }}</span>
      </button>
    </div>
  </Sheet>
</template>
