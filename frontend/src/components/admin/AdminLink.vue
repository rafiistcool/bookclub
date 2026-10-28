<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/client";
import { tp } from "../../i18n";

/**
 * The way to the admin page from Settings, with the number of open reports
 * and catalog requests (`GET /api/admin/summary`), marked when one is
 * overdue. Without the summary it is the plain link.
 */
const { t } = useI18n();

const open = ref(0);
const overdue = ref(0);

onMounted(async () => {
  try {
    const summary = await api.adminSummary();
    open.value = summary.reports.open + summary.edit_requests.open;
    overdue.value = summary.reports.overdue + summary.edit_requests.overdue;
  } catch {
    // The link works without the count.
  }
});
</script>

<template>
  <RouterLink class="btn btn-ghost admin-link" to="/admin">
    {{ t("admin.open") }}
    <span v-if="open" class="admin-count nums" :class="{ overdue: overdue > 0 }">
      <span aria-hidden="true">{{ open }}</span>
      <span class="visually-hidden">{{ tp("admin.waitingCount", open) }}</span>
    </span>
  </RouterLink>
</template>

<style scoped>
.admin-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 1.5rem;
  padding: 0 var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: 700;
  line-height: 1.5rem;
}

.admin-count.overdue {
  color: var(--danger);
  box-shadow: inset 0 0 0 1px var(--danger-border);
}
</style>
