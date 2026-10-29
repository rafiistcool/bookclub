<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api } from "../api/client";
import type { AdminSummary } from "../api/generated";
import AdminInvites from "../components/admin/AdminInvites.vue";
import AdminReports from "../components/admin/AdminReports.vue";
import AdminRequests from "../components/admin/AdminRequests.vue";
import AdminUsers from "../components/admin/AdminUsers.vue";

/**
 * The instance admin's page (`UserOut.is_admin`; the router keeps everyone
 * else out, and the server answers them 404): the catalog queue, reports
 * (covers, profile pictures, books readers added), sign-up invites and
 * suspended accounts. `?tab=` picks the section. Both queue badges
 * come from `GET /api/admin/summary` on mount and again after each decision,
 * marked when something is overdue.
 */
const TABS = ["requests", "reports", "invites", "users"] as const;
type Tab = (typeof TABS)[number];
/** The overdue threshold until the summary says otherwise. */
const DEFAULT_TARGET_HOURS = 72;

const { t } = useI18n();
const route = useRoute();
const router = useRouter();

const tab = computed<Tab>(() => {
  const asked = route.query.tab;
  return (TABS as readonly unknown[]).includes(asked) ? (asked as Tab) : "requests";
});
const summary = ref<AdminSummary | null>(null);
const targetHours = computed(() => summary.value?.target_hours ?? DEFAULT_TARGET_HOURS);

/** The open and overdue counts of a tab's queue; null for invites or before the summary. */
function queueOf(name: Tab) {
  if (!summary.value) return null;
  if (name === "requests") return summary.value.edit_requests;
  if (name === "reports") return summary.value.reports;
  return null;
}

async function loadSummary() {
  try {
    summary.value = await api.adminSummary();
  } catch {
    // The badges stay as they were; each queue shows its own load error.
  }
}

function show(next: Tab) {
  void router.replace({ query: next === "requests" ? {} : { tab: next } });
}

onMounted(loadSummary);
</script>

<template>
  <section class="admin-page">
    <div class="page-head">
      <h1>{{ t("admin.title") }}</h1>
      <p class="lede">{{ t("admin.lede") }}</p>
    </div>

    <div class="segmented admin-tabs" role="group" :aria-label="t('admin.sections')">
      <button v-for="name in TABS" :key="name" type="button" :aria-pressed="tab === name" @click="show(name)">
        {{ t(`admin.tabs.${name}`) }}
        <span
          v-if="queueOf(name)?.open"
          class="seg-count nums"
          :class="{ overdue: (queueOf(name)?.overdue ?? 0) > 0 }"
          :title="(queueOf(name)?.overdue ?? 0) > 0 ? t('admin.overdueCount', { n: queueOf(name)?.overdue }) : undefined"
        >
          {{ queueOf(name)?.open }}
        </span>
      </button>
    </div>

    <AdminRequests v-if="tab === 'requests'" :target-hours="targetHours" @changed="loadSummary" />
    <AdminReports
      v-else-if="tab === 'reports'"
      :target-hours="targetHours"
      :stats="summary?.report_stats ?? null"
      @changed="loadSummary"
    />
    <AdminInvites v-else-if="tab === 'invites'" />
    <AdminUsers v-else />
  </section>
</template>

<style scoped>
.admin-tabs {
  margin-bottom: var(--space-5);
}

/*
 * Four tabs with a count do not fit one line of a phone (at 360 px the
 * fourth one was cut off inside the scrolling bar): two rows of two there.
 */
@media (max-width: 479px) {
  .admin-tabs {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    border-radius: var(--radius-xl);
  }
}

.seg-count.overdue {
  color: var(--danger);
}
</style>
