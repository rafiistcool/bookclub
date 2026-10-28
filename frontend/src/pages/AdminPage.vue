<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AdminInvites from "../components/admin/AdminInvites.vue";
import AdminReports from "../components/admin/AdminReports.vue";
import AdminRequests from "../components/admin/AdminRequests.vue";

/**
 * The instance admin's page (`UserOut.is_admin`; the router keeps everyone
 * else out, and the server answers them 404): the catalog queue, cover
 * reports and sign-up invites. `?tab=` picks the section.
 */
const TABS = ["requests", "reports", "invites"] as const;
type Tab = (typeof TABS)[number];

const { t } = useI18n();
const route = useRoute();
const router = useRouter();

const tab = computed<Tab>(() => {
  const asked = route.query.tab;
  return (TABS as readonly unknown[]).includes(asked) ? (asked as Tab) : "requests";
});
const counts = ref<Partial<Record<Tab, number>>>({});

function show(next: Tab) {
  void router.replace({ query: next === "requests" ? {} : { tab: next } });
}
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
        <span v-if="counts[name]" class="seg-count nums">{{ counts[name] }}</span>
      </button>
    </div>

    <AdminRequests v-if="tab === 'requests'" @count="(n) => (counts.requests = n)" />
    <AdminReports v-else-if="tab === 'reports'" @count="(n) => (counts.reports = n)" />
    <AdminInvites v-else />
  </section>
</template>

<style scoped>
.admin-tabs {
  margin-bottom: var(--space-5);
}
</style>
