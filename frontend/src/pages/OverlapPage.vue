<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import { STATUS_SHORT } from "../constants";
import { useToast } from "../stores/toast";
import type { OverlapBook } from "../types";

const includeReading = ref(false);
const items = ref<OverlapBook[]>([]);
const error = ref("");
const loaded = ref(false);
const toast = useToast();
const nominating = ref<string | null>(null);

function bookHref(key: string) {
  return `https://openlibrary.org${key}`;
}

async function load() {
  loaded.value = false;
  try {
    const result = await api.overlap(includeReading.value);
    items.value = result.items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load overlap";
    items.value = [];
  } finally {
    loaded.value = true;
  }
}

async function nominate(row: OverlapBook) {
  nominating.value = row.book.ol_work_key;
  try {
    await api.nominate({
      ol_work_key: row.book.ol_work_key,
      title: row.book.title,
      authors: row.book.authors,
      cover_id: row.book.cover_id,
      year: row.book.year,
    });
    toast.show("Nominated for next up");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
  } finally {
    nominating.value = null;
  }
}

onMounted(load);
watch(includeReading, load);
</script>

<template>
  <section>
    <p class="fine"><RouterLink to="/friends">← Friends</RouterLink></p>
    <h1 style="margin-top: 8px">TBR overlap</h1>
    <p class="lede">Books more than one of you wants to read.</p>
    <div class="chip-row" style="margin-bottom: 16px">
      <button
        class="chip"
        type="button"
        :aria-pressed="includeReading"
        :class="{ active: includeReading }"
        @click="includeReading = !includeReading"
      >
        Include Reading
      </button>
    </div>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <div v-else-if="items.length === 0" class="empty">
      No shared wants yet. A book only shows here when two or more of you have it on
      Want to read{{ includeReading ? " or Reading" : "" }}.
    </div>
    <div v-else class="book-list">
      <article v-for="row in items" :key="row.book.ol_work_key" class="overlap-row">
        <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="M" />
        <div class="book-meta">
          <h3>{{ row.book.title }}</h3>
          <p class="fine muted">
            {{ row.count }} {{ row.count === 1 ? "person" : "people" }}
          </p>
          <p class="fine">
            <span v-for="(member, index) in row.members" :key="member.username">
              {{ member.username }}
              <span class="muted">({{ STATUS_SHORT[member.status] }})</span>
              <template v-if="index < row.members.length - 1">, </template>
            </span>
          </p>
          <p class="fine">
            <a :href="bookHref(row.book.ol_work_key)" target="_blank" rel="noreferrer">Open Library</a>
          </p>
          <button
            class="btn btn-ghost"
            type="button"
            style="margin-top: 8px"
            :disabled="nominating === row.book.ol_work_key"
            @click="nominate(row)"
          >
            Nominate for next up
          </button>
        </div>
      </article>
    </div>
  </section>
</template>
