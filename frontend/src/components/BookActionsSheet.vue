<script lang="ts">
/** What a row of the book page's "…" sheet asks the page to do. */
export type BookAction =
  | "refresh"
  | "edit"
  | "merge"
  | "report-book"
  | "report-cover"
  | "admin-edit"
  | "admin-edition"
  | "admin-edition-cover";
</script>

<script setup lang="ts">
import { ArrowTopRightOnSquareIcon } from "@heroicons/vue/24/outline";
import { computed, useId } from "vue";
import { useI18n } from "vue-i18n";
import { openLibraryUrl } from "../constants";
import type { BookDetail } from "../types";
import BottomSheet from "./BottomSheet.vue";
import CoverPhotoButton from "./CoverPhotoButton.vue";

/**
 * The book page's "…" sheet: everything about the book that is not reading
 * it, in three groups (the book's data, its cover, and an instance admin's
 * direct edits). It only says what was asked (`act`, `picked`); the page
 * closes it, waits a tick and then opens the next sheet or uploads the file,
 * so there is never a sheet on a sheet and a chosen file is not lost with
 * the closed sheet.
 *
 * The photo rows are real file inputs (`CoverPhotoButton` with `pickOnly`):
 * the tap itself opens the picker, as iOS requires.
 */
const props = defineProps<{
  book: BookDetail;
  /** The title as the page shows it (a withdrawn book's placeholder is translated). */
  title: string;
  admin: boolean;
  /** The cover prompt asks for the reader's photo on the page itself. */
  coverPrompted: boolean;
  withdrawn: boolean;
  refreshing: boolean;
}>();
const emit = defineEmits<{
  close: [];
  act: [action: BookAction];
  picked: [kind: "photo" | "admin", file: File];
}>();

const { t } = useI18n();
const id = useId();

const catalogUrl = computed(() => (props.book.custom ? null : openLibraryUrl(props.book.ol_work_key)));
const hasCover = computed(() => Boolean(props.book.cover_url || props.book.cover_thumb_url));
/** Without a cover, the page keeps the "Add a photo" link under the cloth instead. */
const offersPhoto = computed(() => !props.coverPrompted && hasCover.value);
const offersCoverReport = computed(() => Boolean(props.book.cover_public_id));
const canRefresh = computed(() => !props.book.custom && props.book.can_refresh);
</script>

<template>
  <BottomSheet class="book-actions" :title="title" @close="emit('close')">
    <section class="actions-group" :aria-labelledby="`${id}-book`">
      <h3 :id="`${id}-book`" class="caps subtle">{{ t("book.groupBook") }}</h3>
      <p v-if="book.custom" class="fine subtle">{{ t("book.addedByReader") }}</p>
      <div class="stack actions-rows">
        <a
          v-if="catalogUrl"
          class="btn btn-ghost btn-block"
          :href="catalogUrl"
          target="_blank"
          rel="noreferrer"
          @click="emit('close')"
        >
          {{ t("book.viewOpenLibrary") }}
          <ArrowTopRightOnSquareIcon class="row-icon" aria-hidden="true" />
        </a>
        <button
          v-if="canRefresh"
          class="btn btn-ghost btn-block refresh-details"
          type="button"
          :disabled="refreshing"
          @click="emit('act', 'refresh')"
        >
          {{ refreshing ? t("book.refreshing") : t("book.refreshDetails") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" @click="emit('act', 'edit')">
          {{ t("editRequest.openBook") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" @click="emit('act', 'merge')">
          {{ t("merge.open") }}
        </button>
        <button
          v-if="book.custom && !withdrawn"
          class="btn btn-ghost btn-block report-book"
          type="button"
          @click="emit('act', 'report-book')"
        >
          {{ t("report.action.work") }}
        </button>
      </div>
    </section>

    <section v-if="offersPhoto || offersCoverReport" class="actions-group" :aria-labelledby="`${id}-cover`">
      <h3 :id="`${id}-cover`" class="caps subtle">{{ t("book.groupCover") }}</h3>
      <div class="stack actions-rows">
        <CoverPhotoButton
          v-if="offersPhoto"
          pick-only
          look="row"
          class="suggest-photo"
          :work-id="book.id"
          :edition-id="book.edition_id"
          :has-cover="hasCover"
          @picked="(file) => emit('picked', 'photo', file)"
        />
        <button
          v-if="offersCoverReport"
          class="btn btn-ghost btn-block report-cover"
          type="button"
          @click="emit('act', 'report-cover')"
        >
          {{ t("cover.report") }}
        </button>
      </div>
    </section>

    <section v-if="admin" class="actions-group" :aria-labelledby="`${id}-admin`">
      <h3 :id="`${id}-admin`" class="caps subtle">{{ t("book.groupAdmin") }}</h3>
      <div class="stack actions-rows">
        <button class="btn btn-ghost btn-block" type="button" @click="emit('act', 'admin-edit')">
          {{ t("admin.editAsAdmin") }}
        </button>
        <button
          v-if="book.edition_id != null"
          class="btn btn-ghost btn-block"
          type="button"
          @click="emit('act', 'admin-edition')"
        >
          {{ t("admin.editEditionAsAdmin") }}
        </button>
        <CoverPhotoButton
          admin
          pick-only
          look="row"
          class="admin-set-cover"
          :label="t('admin.setCover')"
          :work-id="book.id"
          :edition-id="null"
          :has-cover="hasCover"
          @picked="(file) => emit('picked', 'admin', file)"
        />
        <button
          class="btn btn-ghost btn-block admin-set-edition-cover"
          type="button"
          @click="emit('act', 'admin-edition-cover')"
        >
          {{ t("admin.setEditionCover") }}
        </button>
      </div>
    </section>
  </BottomSheet>
</template>

<style scoped>
.actions-group + .actions-group {
  margin-top: var(--space-5);
}

.actions-group h3 {
  margin: 0 0 var(--space-2);
}

.actions-group > .fine {
  margin: 0 0 var(--space-2);
}

.actions-rows {
  gap: var(--space-2);
}

.row-icon {
  width: 18px;
  height: 18px;
  flex: none;
}
</style>
