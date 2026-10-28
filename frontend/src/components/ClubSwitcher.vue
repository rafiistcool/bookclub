<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { switchClubPath, useClubs } from "../stores/clubs";

/** Moves to another of the viewer's clubs, staying in the same section. */
defineProps<{ slug: string }>();

const { t } = useI18n();
const clubs = useClubs();
const route = useRoute();
const router = useRouter();

function go(event: Event) {
  const next = (event.target as HTMLSelectElement).value;
  if (next) void router.push(switchClubPath(route.path, next));
}
</script>

<template>
  <div class="club-switcher">
    <label v-if="clubs.items.length > 1" class="switcher-field">
      <span class="visually-hidden">{{ t("club.switch") }}</span>
      <select :value="slug" @change="go">
        <option v-for="club in clubs.items" :key="club.slug" :value="club.slug">{{ club.name }}</option>
      </select>
    </label>
    <RouterLink class="btn btn-ghost btn-sm" to="/clubs">{{ t("club.allClubs") }}</RouterLink>
  </div>
</template>

<style scoped>
.club-switcher {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}

.switcher-field select {
  min-height: var(--tap);
  max-width: 16rem;
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-weight: 600;
}
</style>
