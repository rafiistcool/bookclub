<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { LEGAL_DOCS, legalPath } from "../legal";
import { useService } from "../stores/service";

/**
 * Imprint · Privacy · Terms: the operator's legal texts, wherever a visitor
 * may need them (the auth pages, the app shell). Only the documents the
 * server provides (`/api/config.legal`) are linked; without any, nothing shows.
 */
const { t } = useI18n();
const service = useService();

const docs = computed(() => LEGAL_DOCS.filter((doc) => service.hasLegal(doc)));
</script>

<template>
  <nav v-if="docs.length" class="legal-links fine" :aria-label="t('legal.nav')">
    <RouterLink v-for="doc in docs" :key="doc" :to="legalPath(doc)">{{ t(`legal.short.${doc}`) }}</RouterLink>
  </nav>
</template>

<style scoped>
.legal-links {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1) var(--space-4);
}

.legal-links a {
  color: var(--text-muted);
  text-decoration: none;
}

.legal-links a:hover {
  color: var(--text);
  text-decoration: underline;
}
</style>
