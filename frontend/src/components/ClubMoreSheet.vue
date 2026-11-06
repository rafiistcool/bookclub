<script setup lang="ts">
import {
  ChartBarIcon,
  ChevronRightIcon,
  Cog6ToothIcon,
  HeartIcon,
  SparklesIcon,
  UserPlusIcon,
} from "@heroicons/vue/24/outline";
import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import BottomSheet from "./BottomSheet.vue";

/**
 * The club's "More" tab: the quieter sections (shared wants, activity, the
 * year), and for admins the two places on the members page they come for,
 * reached by their anchors.
 */
const props = defineProps<{ slug: string; canManage: boolean }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const route = useRoute();

type Row = { to: string; label: string; icon: Component };

const sections = computed<Row[]>(() => {
  const base = `/c/${props.slug}`;
  return [
    { to: `${base}/overlap`, label: t("clubNav.overlap"), icon: HeartIcon },
    { to: `${base}/events`, label: t("clubNav.events"), icon: SparklesIcon },
    { to: `${base}/stats`, label: t("clubNav.stats"), icon: ChartBarIcon },
  ];
});

const adminRows = computed<Row[]>(() => {
  const members = `/c/${props.slug}/members`;
  return [
    { to: `${members}#invites`, label: t("clubNav.invite"), icon: UserPlusIcon },
    { to: `${members}#club-settings`, label: t("clubSettings.heading"), icon: Cog6ToothIcon },
  ];
});

function isHere(to: string): boolean {
  return route.path === to || route.path.startsWith(`${to}/`);
}
</script>

<template>
  <BottomSheet class="club-more" :title="t('clubNav.more')" @close="emit('close')">
    <ul class="more-list">
      <li v-for="row in sections" :key="row.to">
        <RouterLink
          class="more-row"
          :class="{ current: isHere(row.to) }"
          :to="row.to"
          :aria-current="isHere(row.to) ? 'page' : undefined"
          @click="emit('close')"
        >
          <component :is="row.icon" class="more-icon" aria-hidden="true" />
          <span class="more-label">{{ row.label }}</span>
          <ChevronRightIcon class="more-chevron" aria-hidden="true" />
        </RouterLink>
      </li>
    </ul>
    <ul v-if="canManage" class="more-list more-admin">
      <li v-for="row in adminRows" :key="row.to">
        <RouterLink class="more-row" :to="row.to" @click="emit('close')">
          <component :is="row.icon" class="more-icon" aria-hidden="true" />
          <span class="more-label">{{ row.label }}</span>
          <ChevronRightIcon class="more-chevron" aria-hidden="true" />
        </RouterLink>
      </li>
    </ul>
  </BottomSheet>
</template>

<style scoped>
.more-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.more-list li + li {
  border-top: 1px solid var(--border);
}

.more-admin {
  margin-top: var(--space-3);
  border-top: 1px solid var(--border-strong);
}

.more-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 52px;
  padding: 0 var(--space-1);
  color: var(--text);
  font-weight: 600;
  text-decoration: none;
  border-radius: var(--radius-sm);
  transition: transform var(--dur-fast) var(--ease-out);
}

.more-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .more-row:hover .more-label {
    color: var(--accent);
  }
}

.more-icon {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--text-muted);
}

.more-label {
  flex: 1;
  min-width: 0;
}

.more-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}

.current .more-label,
.current .more-icon {
  color: var(--accent);
}
</style>
