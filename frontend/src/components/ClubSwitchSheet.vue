<script setup lang="ts">
import { CheckIcon, PlusIcon, Squares2X2Icon } from "@heroicons/vue/24/outline";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { tp } from "../i18n";
import { isClubAdmin, switchClubPath, useClubs } from "../stores/clubs";
import BottomSheet from "./BottomSheet.vue";

/**
 * The club name's sheet: the viewer's clubs (from the session, no request),
 * each leading to the same section of that club, then the club list and
 * "create a club". It shows with one club too, so the list stays reachable.
 */
defineProps<{ slug: string }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const clubs = useClubs();
const route = useRoute();
</script>

<template>
  <BottomSheet class="club-switch" :title="t('club.yourClubs')" @close="emit('close')">
    <ul class="switch-list">
      <li v-for="row in clubs.items" :key="row.slug">
        <RouterLink
          class="switch-row club-choice"
          :class="{ current: row.slug === slug }"
          :to="switchClubPath(route.path, row.slug)"
          :aria-current="row.slug === slug ? 'true' : undefined"
          @click="emit('close')"
        >
          <span class="switch-text">
            <span class="switch-name">{{ row.name }}</span>
            <span class="finer subtle">
              {{ tp("club.memberCount", row.member_count)
              }}<template v-if="isClubAdmin(row.role)"> · {{ t(`roles.${row.role}`) }}</template>
            </span>
          </span>
          <CheckIcon v-if="row.slug === slug" class="switch-check" aria-hidden="true" />
        </RouterLink>
      </li>
    </ul>
    <ul class="switch-list switch-more">
      <li>
        <RouterLink class="switch-row" to="/clubs" @click="emit('close')">
          <Squares2X2Icon class="switch-icon" aria-hidden="true" />
          <span class="switch-label">{{ t("club.allClubs") }}</span>
        </RouterLink>
      </li>
      <li>
        <RouterLink class="switch-row" to="/clubs/new" @click="emit('close')">
          <PlusIcon class="switch-icon" aria-hidden="true" />
          <span class="switch-label">{{ t("clubs.create") }}</span>
        </RouterLink>
      </li>
    </ul>
  </BottomSheet>
</template>

<style scoped>
.switch-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.switch-list li + li {
  border-top: 1px solid var(--border);
}

.switch-more {
  margin-top: var(--space-3);
  border-top: 1px solid var(--border-strong);
}

.switch-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 56px;
  padding: var(--space-2) var(--space-1);
  color: var(--text);
  text-decoration: none;
  border-radius: var(--radius-sm);
  transition: transform var(--dur-fast) var(--ease-out);
}

.switch-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .switch-row:hover .switch-name,
  .switch-row:hover .switch-label {
    color: var(--accent);
  }
}

.switch-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.switch-name {
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 600;
  line-height: var(--leading-snug);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.current .switch-name {
  color: var(--accent);
}

.switch-check {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--accent);
}

.switch-icon {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--text-muted);
}

.switch-label {
  font-weight: 600;
}

.switch-more .switch-row {
  min-height: 52px;
}
</style>
