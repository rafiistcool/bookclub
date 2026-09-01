<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import Skeleton from "../components/Skeleton.vue";
import { STATUS_SHORT } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import type { OverlapBook } from "../types";

const includeReading = ref(false);
const items = ref<OverlapBook[] | null>(null);
const error = ref("");
const expanded = ref<string | null>(null);
const flow = useFlow();

async function load() {
  items.value = null;
  try {
    items.value = (await api.overlap(includeReading.value)).items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load overlap";
    items.value = [];
  }
}

onMounted(load);
watch(includeReading, load);
</script>

<template>
  <section aria-label="Shared to-read">
    <p class="lede fine" style="margin-bottom: 12px">Books more than one of you wants to read — the natural shortlist for the next vote.</p>
    <div class="toggle-row" style="border: 0; padding: 0 0 12px">
      <span class="fine">Include books someone is already reading</span>
      <button class="switch" type="button" role="switch" :aria-checked="includeReading" @click="includeReading = !includeReading" />
    </div>
    <p v-if="error" class="error">{{ error }}</p>
    <Skeleton v-else-if="!items" kind="cards" :count="3" />
    <div v-else-if="items.length === 0" class="empty">
      <p>No shared wants yet.</p>
      <p class="fine">A book shows here once two or more of you have it on Want to read{{ includeReading ? " or Reading" : "" }}.</p>
    </div>
    <div v-else class="list">
      <article v-for="row in items" :key="row.book.ol_work_key" class="row-item">
        <button type="button" style="all: unset; cursor: pointer" @click="flow.open({ kind: 'details', book: toRef(row.book) })">
          <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="sm" />
        </button>
        <div class="row-body">
          <h3 class="clamp-1">{{ row.book.title }}</h3>
          <p class="clamp-1">{{ row.book.authors }}</p>
          <button
            type="button"
            class="fine"
            style="all: unset; cursor: pointer; display: inline-flex; align-items: center; gap: 6px; margin-top: 4px"
            :aria-expanded="expanded === row.book.ol_work_key"
            @click="expanded = expanded === row.book.ol_work_key ? null : row.book.ol_work_key"
          >
            <span class="avatar-stack">
              <Avatar v-for="member in row.members" :key="member.username" :username="member.username" size="sm" />
            </span>
            <span class="muted">{{ row.count }} people</span>
          </button>
          <p v-if="expanded === row.book.ol_work_key" class="fine muted" style="margin-top: 4px">
            <template v-for="(member, index) in row.members" :key="member.username">
              {{ member.username }} <span class="faint">({{ STATUS_SHORT[member.status] }})</span><template v-if="index < row.members.length - 1">, </template>
            </template>
          </p>
        </div>
        <button class="btn btn-ghost btn-sm" type="button" @click="flow.nominate(toRef(row.book))">Nominate</button>
      </article>
    </div>
  </section>
</template>
