<script setup lang="ts">
import { ChevronRightIcon } from "@heroicons/vue/24/outline";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { LEGAL_DOCS, legalPath, useService } from "../../stores/service";
import BottomSheet from "../BottomSheet.vue";

/**
 * The operator's legal texts and contact address. Settings has no shell
 * footer, so this sheet is where they are; it only opens when the server has
 * a text or a contact (the row hides otherwise).
 */
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const service = useService();
const docs = computed(() => LEGAL_DOCS.filter((doc) => service.hasLegal(doc)));
</script>

<template>
  <BottomSheet class="legal-sheet" :title="t('legal.settingsTitle')" @close="emit('close')">
    <ul v-if="docs.length" class="legal-list">
      <li v-for="doc in docs" :key="doc">
        <RouterLink class="legal-row" :to="legalPath(doc)" @click="emit('close')">
          <span class="legal-label">{{ t(`legal.title.${doc}`) }}</span>
          <ChevronRightIcon class="legal-chevron" aria-hidden="true" />
        </RouterLink>
      </li>
    </ul>
    <p v-if="service.contact_email" class="fine muted legal-contact">
      <i18n-t keypath="legal.contact" tag="span" scope="global">
        <template #email>
          <a :href="`mailto:${service.contact_email}`">{{ service.contact_email }}</a>
        </template>
      </i18n-t>
    </p>
  </BottomSheet>
</template>

<style scoped>
.legal-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.legal-list li + li {
  border-top: 1px solid var(--border);
}

.legal-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 52px;
  padding: 0;
  border-radius: var(--radius-sm);
  color: var(--text);
  font-weight: 600;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.legal-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .legal-row:hover .legal-label {
    color: var(--accent);
  }
}

.legal-label {
  flex: 1;
  min-width: 0;
}

.legal-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}

.legal-contact {
  margin-top: var(--space-4);
  overflow-wrap: anywhere;
}

.legal-list + .legal-contact {
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
}
</style>
