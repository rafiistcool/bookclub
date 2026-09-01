<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import NavIcon from "../components/NavIcon.vue";
import Skeleton from "../components/Skeleton.vue";
import { useFlow, toRef } from "../stores/flow";
import { useToast } from "../stores/toast";
import type { Quote } from "../types";

const route = useRoute();
const flow = useFlow();
const toast = useToast();
const quotes = ref<Quote[] | null>(null);
const error = ref("");
const editing = ref<Quote | null>(null);
const draft = ref("");
const pageDraft = ref<number | "">("");

const work = computed(() => (typeof route.query.work === "string" ? route.query.work : undefined));
const title = computed(() => (work.value && quotes.value?.[0]?.book.title) || null);

async function load() {
  quotes.value = null;
  try {
    quotes.value = (await api.quotes({ work: work.value, limit: 200 })).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load quotes";
    quotes.value = [];
  }
}

function startEdit(quote: Quote) {
  editing.value = quote;
  draft.value = quote.body;
  pageDraft.value = quote.page ?? "";
}

async function saveEdit() {
  const quote = editing.value;
  if (!quote || !draft.value.trim()) return;
  try {
    const updated = await api.editQuote(quote.id, {
      body: draft.value.trim(),
      page: pageDraft.value === "" ? null : Number(pageDraft.value),
    });
    quotes.value = (quotes.value ?? []).map((row) => (row.id === updated.id ? updated : row));
    editing.value = null;
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save");
  }
}

async function remove(quote: Quote) {
  const before = quotes.value ?? [];
  quotes.value = before.filter((row) => row.id !== quote.id);
  try {
    await api.deleteQuote(quote.id);
    toast.show("Quote deleted");
  } catch (err) {
    quotes.value = before;
    toast.show(err instanceof ApiError ? err.message : "Could not delete");
  }
}

onMounted(load);
watch(work, load);
</script>

<template>
  <section aria-label="Quotes">
    <p v-if="title" class="lede fine" style="margin-bottom: 12px">
      From <span class="strong">{{ title }}</span> · <RouterLink to="/quotes">all quotes</RouterLink>
    </p>
    <p v-if="error" class="error">{{ error }}</p>
    <Skeleton v-else-if="!quotes" kind="cards" :count="3" />
    <div v-else-if="quotes.length === 0" class="empty">
      <NavIcon name="quote" :size="28" />
      <p>No quotes saved yet.</p>
      <p class="fine">Open any book and tap Quote to keep a line.</p>
    </div>
    <div v-else class="list">
      <article v-for="quote in quotes" :key="quote.id" class="card">
        <template v-if="editing?.id === quote.id">
          <textarea v-model="draft" class="input" rows="4" maxlength="1000" />
          <input v-model="pageDraft" class="input" type="number" inputmode="numeric" placeholder="Page" style="margin-top: 8px; width: 120px" />
          <div class="actions" style="margin-top: 8px">
            <button class="btn btn-primary btn-sm" type="button" @click="saveEdit">Save</button>
            <button class="btn btn-ghost btn-sm" type="button" @click="editing = null">Cancel</button>
          </div>
        </template>
        <template v-else>
          <blockquote class="serif" style="margin: 0; font-size: var(--text-lg); line-height: 1.4; font-style: italic">“{{ quote.body }}”</blockquote>
          <div style="display: flex; align-items: center; gap: 10px; margin-top: 12px">
            <button type="button" style="all: unset; cursor: pointer" @click="flow.open({ kind: 'details', book: toRef(quote.book) })">
              <BookCover :title="quote.book.title" :cover-id="quote.book.cover_id" size="xs" />
            </button>
            <div style="flex: 1; min-width: 0" class="fine">
              <p class="clamp-1 strong">{{ quote.book.title }}<span v-if="quote.page" class="muted"> · p. {{ quote.page }}</span></p>
              <p class="muted" style="display: flex; align-items: center; gap: 6px"><Avatar :username="quote.author" size="sm" /> {{ quote.author }} · {{ quote.created_label }}</p>
            </div>
            <template v-if="quote.mine">
              <button class="icon-btn" type="button" aria-label="Edit quote" @click="startEdit(quote)"><NavIcon name="edit" :size="18" /></button>
              <button class="icon-btn" type="button" aria-label="Delete quote" style="color: var(--danger)" @click="remove(quote)"><NavIcon name="trash" :size="18" /></button>
            </template>
          </div>
        </template>
      </article>
    </div>
  </section>
</template>
