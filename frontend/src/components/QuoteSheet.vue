<script setup lang="ts">
import { ref } from "vue";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import type { BookRef } from "../types";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  book: BookRef;
}>();

const emit = defineEmits<{
  close: [];
  saved: [];
}>();

const body = ref("");
// type="number" inputs hand v-model a number (or "" when cleared).
const page = ref<number | "">("");
const pending = ref(false);
const toast = useToast();

async function save() {
  if (!body.value.trim() || pending.value) return;
  pending.value = true;
  try {
    const pageNumber = page.value === "" ? null : Number(page.value);
    await api.addQuote({
      ...props.book,
      body: body.value.trim(),
      page: pageNumber !== null && Number.isFinite(pageNumber) ? pageNumber : null,
    });
    toast.show("Quote saved");
    emit("saved");
    emit("close");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that quote");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <Sheet title="Save a quote" :subtitle="book.title" @close="emit('close')">
    <label class="field">
      <span>The line</span>
      <textarea v-model="body" rows="4" maxlength="1000" placeholder="Paste or type the passage" data-autofocus />
      <span class="field-hint">{{ body.length }}/1000</span>
    </label>
    <label class="field">
      <span>Page <span class="faint">(optional)</span></span>
      <input v-model="page" type="number" inputmode="numeric" min="0" placeholder="e.g. 142" />
    </label>
    <template #foot>
      <button class="btn btn-primary" type="button" :disabled="!body.trim() || pending" @click="save">
        {{ pending ? "Saving…" : "Save quote" }}
      </button>
    </template>
  </Sheet>
</template>
