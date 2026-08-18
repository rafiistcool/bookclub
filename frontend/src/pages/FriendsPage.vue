<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import type { Member } from "../types";

const members = ref<Member[]>([]);
const error = ref("");
const loaded = ref(false);

onMounted(async () => {
  try {
    members.value = await api.members();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load friends";
  } finally {
    loaded.value = true;
  }
});
</script>

<template>
  <section>
    <h1>Friends</h1>
    <p class="lede">See what everyone else is reading.</p>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <div v-else-if="members.length === 0" class="empty">
      You’re the first one here. Mint an invite and send it to a friend.
      <div style="margin-top: 12px">
        <RouterLink class="btn" to="/invites">Create an invite</RouterLink>
      </div>
    </div>
    <div v-else class="book-list">
      <RouterLink
        v-for="member in members"
        :key="member.username"
        class="member-card"
        :to="`/friends/${member.username}`"
      >
        <div>
          <h3>{{ member.username }}</h3>
          <p class="muted fine">
            {{ member.currently_reading_count }} currently reading
          </p>
        </div>
        <div class="preview-row">
          <BookCover
            v-for="book in member.currently_reading_preview"
            :key="book.title"
            :title="book.title"
            :cover-id="book.cover_id"
            size="S"
            tiny
          />
        </div>
      </RouterLink>
    </div>
  </section>
</template>
