<script setup lang="ts">
import { ref } from "vue";
import type { FinishNote, Status } from "../constants";
import Sheet from "./Sheet.vue";
import StarInput from "./StarInput.vue";

const props = defineProps<{
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

function save() {
  if (props.status === "finished") {
    emit("confirm", { rating: rating.value, take: take.value.trim() });
    return;
  }
  emit("confirm", { dnf_reason: reason.value.trim() });
}
</script>

<template>
  <Sheet :title="status === 'finished' ? 'Finished' : 'Did not finish'" :subtitle="bookTitle" @close="emit('close')">
    <template v-if="status === 'finished'">
      <div class="field">
        <span class="field-label">Rating</span>
        <StarInput v-model="rating" />
      </div>
      <label class="field">
        <span>One-line take</span>
        <input v-model="take" type="text" maxlength="140" placeholder="Optional — what stuck with you?" data-autofocus />
        <span class="field-hint">{{ take.length }}/140</span>
      </label>
    </template>
    <label v-else class="field">
      <span>Why didn’t you finish?</span>
      <input v-model="reason" type="text" maxlength="200" placeholder="Optional" data-autofocus />
    </label>
    <template #foot>
      <button class="btn btn-primary" type="button" @click="save">Save</button>
    </template>
  </Sheet>
</template>
