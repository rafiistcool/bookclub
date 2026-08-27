<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { starLabel, type FinishNote, type Status } from "../constants";

const props = defineProps<{
  title: string;
  bookTitle: string;
  status: Extract<Status, "finished" | "did_not_finish">;
  rating?: number | null;
  take?: string;
  dnfReason?: string;
}>();

const emit = defineEmits<{
  confirm: [note: FinishNote];
  close: [];
}>();

const rating = ref<number | null>(props.rating ?? null);
const take = ref(props.take ?? "");
const reason = ref(props.dnfReason ?? "");

function onKey(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

function save() {
  if (props.status === "finished") {
    emit("confirm", { rating: rating.value, take: take.value.trim() });
    return;
  }
  emit("confirm", { dnf_reason: reason.value.trim() });
}

onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="sheet-backdrop" @click.self="emit('close')">
    <div class="sheet" role="dialog" aria-modal="true" :aria-label="title">
      <h2>{{ title }}</h2>
      <p class="muted" style="margin-bottom: 14px">{{ bookTitle }}</p>
      <template v-if="status === 'finished'">
        <p class="field">
          <span>Rating</span>
          <span class="star-row">
            <button
              v-for="value in 5"
              :key="value"
              class="star-btn"
              type="button"
              :aria-pressed="rating === value"
              :aria-label="`${value} of 5`"
              @click="rating = rating === value ? null : value"
            >
              {{ rating && rating >= value ? "★" : "☆" }}
            </button>
          </span>
          <span v-if="rating" class="fine muted">{{ starLabel(rating) }}</span>
        </p>
        <label class="field">
          <span>One-line take</span>
          <input v-model="take" type="text" maxlength="140" placeholder="Optional" />
        </label>
      </template>
      <label v-else class="field">
        <span>Why didn’t you finish? (optional)</span>
        <input v-model="reason" type="text" maxlength="200" placeholder="Optional" />
      </label>
      <button class="btn btn-primary" type="button" @click="save">Save</button>
      <button class="btn btn-ghost" type="button" style="width: 100%; margin-top: 10px" @click="emit('close')">
        Cancel
      </button>
    </div>
  </div>
</template>
