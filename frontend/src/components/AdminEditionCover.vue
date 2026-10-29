<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { EditionOut } from "../api/generated";
import { editionLine } from "../editions";
import BookCover from "./BookCover.vue";
import BottomSheet from "./BottomSheet.vue";
import CoverPhotoButton from "./CoverPhotoButton.vue";

/**
 * An instance admin's direct cover for any edition of the book (#74): the
 * link opens the list of the work's editions, each with its cover and an
 * upload. The picture replaces that edition's cover at once
 * (`PUT /api/admin/works/{work_id}/cover` with its `edition_id`). The
 * viewer's own edition is marked; the admin need not own the one they fix.
 */
const props = defineProps<{ workId: number; editionId: number | null; title: string }>();
const emit = defineEmits<{ covered: [] }>();

const { t } = useI18n();
const open = ref(false);
const editions = ref<EditionOut[] | null>(null);
const error = ref("");

async function show() {
  open.value = true;
  editions.value = null;
  error.value = "";
  try {
    editions.value = (await api.editions(props.workId)).items;
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("editions.loadFailed");
  }
}

function covered() {
  open.value = false;
  emit("covered");
}
</script>

<template>
  <span class="admin-edition-cover">
    <button class="text-btn" type="button" @click="show">{{ t("admin.setEditionCover") }}</button>
    <BottomSheet v-if="open" :title="t('admin.editionCover.title')" @close="open = false">
      <p class="fine muted">{{ t("admin.editionCover.blurb") }}</p>
      <p v-if="error" class="error">{{ error }}</p>
      <div v-else-if="!editions" class="skeleton editions-skeleton" aria-hidden="true" />
      <p v-else-if="editions.length === 0" class="fine subtle">{{ t("admin.editionCover.none") }}</p>
      <ul v-else class="edition-covers">
        <li v-for="edition in editions" :key="edition.id" class="edition-cover-row">
          <BookCover
            :book="{ title: edition.title || title, cover_url: edition.cover_url, cover_thumb_url: edition.cover_thumb_url }"
            size="xs"
          />
          <span class="edition-cover-text fine">
            <strong v-if="edition.title">{{ edition.title }}</strong>
            <span>{{ editionLine(edition) }}</span>
            <span v-if="edition.id === editionId" class="subtle">{{ t("admin.editionCover.yours") }}</span>
          </span>
          <CoverPhotoButton
            admin
            look="ghost"
            :label="t('admin.editionCover.upload')"
            :work-id="workId"
            :edition-id="edition.id"
            :has-cover="Boolean(edition.cover_url || edition.cover_thumb_url)"
            @covered="covered"
          />
        </li>
      </ul>
    </BottomSheet>
  </span>
</template>

<style scoped>
/* The link sits in a line of small text links; the sheet reads at body size. */
.admin-edition-cover :deep(.sheet) {
  font-size: var(--text-md);
  text-align: left;
}

.editions-skeleton {
  height: 120px;
}

.edition-covers {
  list-style: none;
  margin: var(--space-3) 0 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.edition-cover-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) 0;
  border-bottom: 1px solid var(--border);
}

.edition-cover-text {
  display: grid;
  flex: 1;
  min-width: 0;
  gap: 2px;
  overflow-wrap: anywhere;
}

.edition-cover-row :deep(.cover-photo) {
  flex-shrink: 0;
}
</style>
