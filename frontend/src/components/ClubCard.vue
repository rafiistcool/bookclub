<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { relativeDay } from "../constants";
import { tp } from "../i18n";
import { isClubAdmin } from "../stores/clubs";
import type { ClubOut } from "../types";
import BookCover from "./BookCover.vue";

/** One of the viewer's clubs: its name and what it is reading now. */
defineProps<{ club: ClubOut }>();

const { t } = useI18n();
</script>

<template>
  <RouterLink class="club-card" :to="`/c/${club.slug}`">
    <span class="club-card-cover">
      <BookCover v-if="club.current_pick" :book="club.current_pick.book" size="sm" />
      <span v-else class="club-card-blank" aria-hidden="true">{{ club.name.charAt(0).toUpperCase() }}</span>
    </span>
    <span class="club-card-meta">
      <strong class="club-card-name">{{ club.name }}</strong>
      <span class="finer subtle">
        {{ tp("club.memberCount", club.member_count) }}
        <template v-if="isClubAdmin(club.role)"> · {{ t(`roles.${club.role}`) }}</template>
      </span>
      <span v-if="club.current_pick" class="fine club-card-pick">
        {{ t("clubs.reading", { title: club.current_pick.book.title }) }}
      </span>
      <span v-else class="fine subtle">{{ t("clubs.noPick") }}</span>
      <span v-if="club.current_pick?.meeting_at" class="finer subtle">
        {{ t("clubs.meeting", { when: relativeDay(club.current_pick.meeting_at, club.timezone) }) }}
      </span>
    </span>
  </RouterLink>
</template>

<style scoped>
/* A light row: the list around it draws the hairlines (ClubsPage). */
.club-card {
  display: flex;
  gap: var(--space-3);
  padding: var(--space-3) 0;
  color: inherit;
  text-decoration: none;
  min-width: 0;
  transition: transform var(--dur-fast) var(--ease-out);
}

.club-card:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .club-card:hover .club-card-name {
    color: var(--accent);
  }
}

.club-card-cover {
  flex: 0 0 auto;
}

.club-card-blank {
  display: grid;
  place-items: center;
  width: 52px;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-sm);
  background: var(--accent-soft);
  color: var(--accent-hover);
  font-family: var(--serif);
  font-size: var(--text-xl);
  font-weight: 700;
}

.club-card-meta {
  display: grid;
  gap: 2px;
  align-content: start;
  min-width: 0;
}

.club-card-name {
  font-family: var(--serif);
  font-size: var(--text-lg);
  line-height: var(--leading-snug);
  overflow-wrap: anywhere;
}

.club-card-pick {
  overflow-wrap: anywhere;
}
</style>
