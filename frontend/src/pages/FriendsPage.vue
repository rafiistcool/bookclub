<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import ActivityFeed from "../components/ActivityFeed.vue";
import Avatar from "../components/Avatar.vue";
import NavIcon from "../components/NavIcon.vue";
import { useSession } from "../stores/session";
import type { Member } from "../types";

const session = useSession();
const members = ref<Member[] | null>(null);
const error = ref("");

onMounted(async () => {
  try {
    members.value = await api.members();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load members";
  }
});
</script>

<template>
  <section aria-label="Club">
    <div class="member-chips" aria-label="Members">
      <RouterLink v-if="session.user" class="member-chip" :to="`/friends/${session.user.username}`">
        <Avatar :username="session.user.username" size="lg" />
        <span>You</span>
      </RouterLink>
      <RouterLink v-for="member in members ?? []" :key="member.username" class="member-chip" :to="`/friends/${member.username}`">
        <Avatar :username="member.username" size="lg" />
        <span class="clamp-1">{{ member.username }}</span>
        <span class="tiny faint">{{ member.currently_reading_count }} reading</span>
      </RouterLink>
      <RouterLink class="member-chip" to="/settings#invites">
        <span class="avatar" style="--avatar-bg: var(--paper-3); color: var(--ink-soft); --size: 48px"><NavIcon name="plus" /></span>
        <span>Invite</span>
      </RouterLink>
    </div>
    <p v-if="error" class="error fine">{{ error }}</p>

    <div class="chip-row" style="margin: 4px 0 12px" aria-label="Club pages">
      <RouterLink class="chip" to="/overlap">Shared to-read</RouterLink>
      <RouterLink class="chip" to="/stats"><NavIcon name="chart" :size="16" /> Year in review</RouterLink>
      <RouterLink class="chip" to="/quotes"><NavIcon name="quote" :size="16" /> Quotes</RouterLink>
    </div>

    <div v-if="members && members.length === 0" class="card" style="margin-bottom: 16px">
      <p class="strong">You’re the first one here.</p>
      <p class="fine muted" style="margin: 4px 0 10px">Mint an invite in Settings and send it to a friend. The feed fills up once they add books.</p>
      <RouterLink class="btn btn-primary btn-sm" to="/settings#invites">Create an invite</RouterLink>
    </div>

    <section aria-labelledby="feed-title">
      <div class="section-title">
        <h2 id="feed-title">Activity</h2>
      </div>
      <ActivityFeed :limit="30" />
    </section>
  </section>
</template>
