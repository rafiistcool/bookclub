<script lang="ts">
/** What a row of the book page's "…" sheet asks the page to do. */
export type BookAction =
  | "refresh"
  | "edit"
  | "merge"
  | "report-book"
  | "report-cover"
  | "choose-cover"
  | "admin-edit"
  | "admin-edition"
  | "admin-edition-cover";
</script>

<script setup lang="ts">
import {
  ArrowPathIcon,
  ArrowTopRightOnSquareIcon,
  BookOpenIcon,
  CameraIcon,
  DocumentDuplicateIcon,
  FlagIcon,
  PencilSquareIcon,
  PhotoIcon,
  Squares2X2Icon,
  WrenchScrewdriverIcon,
} from "@heroicons/vue/24/outline";
import { computed, useId } from "vue";
import { useI18n } from "vue-i18n";
import { openLibraryUrl } from "../constants";
import type { BookDetail } from "../types";
import BottomSheet from "./BottomSheet.vue";
import CoverPhotoButton from "./CoverPhotoButton.vue";

/**
 * The book page's "…" sheet: everything about the book that is not reading
 * it, in three groups (the book's data, its cover, and an instance admin's
 * direct edits), as light rows like the club's "More" sheet. It only says what was asked (`act`, `picked`); the page
 * closes it, waits a tick and then opens the next sheet or uploads the file,
 * so there is never a sheet on a sheet and a chosen file is not lost with
 * the closed sheet.
 *
 * "Choose cover" (the reader's own cover, `CoverChooserSheet`) is there only
 * while the book is on the reader's shelf: the choice belongs to the entry.
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
const offersChoice = computed(() => props.book.shelf_id != null && !props.withdrawn);
</script>

<template>
  <BottomSheet class="book-actions" :title="title" @close="emit('close')">
    <section class="actions-group" :aria-labelledby="`${id}-book`">
      <h3 :id="`${id}-book`" class="group-head">{{ t("book.groupBook") }}</h3>
      <p v-if="book.custom" class="fine subtle">{{ t("book.addedByReader") }}</p>
      <ul class="sheet-rows">
        <li v-if="catalogUrl">
          <a class="sheet-row" :href="catalogUrl" target="_blank" rel="noreferrer" @click="emit('close')">
            <BookOpenIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("book.viewOpenLibrary") }}</span>
            <ArrowTopRightOnSquareIcon class="row-end" aria-hidden="true" />
          </a>
        </li>
        <li v-if="canRefresh">
          <button
            class="sheet-row refresh-details"
            type="button"
            :disabled="refreshing"
            @click="emit('act', 'refresh')"
          >
            <ArrowPathIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ refreshing ? t("book.refreshing") : t("book.refreshDetails") }}</span>
          </button>
        </li>
        <li>
          <button class="sheet-row" type="button" @click="emit('act', 'edit')">
            <PencilSquareIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("editRequest.openBook") }}</span>
          </button>
        </li>
        <li>
          <button class="sheet-row" type="button" @click="emit('act', 'merge')">
            <DocumentDuplicateIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("merge.open") }}</span>
          </button>
        </li>
        <li v-if="book.custom && !withdrawn">
          <button class="sheet-row report-book" type="button" @click="emit('act', 'report-book')">
            <FlagIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("report.action.work") }}</span>
          </button>
        </li>
      </ul>
    </section>

    <section v-if="offersChoice || offersPhoto || offersCoverReport" class="actions-group" :aria-labelledby="`${id}-cover`">
      <h3 :id="`${id}-cover`" class="group-head">{{ t("book.groupCover") }}</h3>
      <ul class="sheet-rows">
        <li v-if="offersChoice">
          <button class="sheet-row choose-cover" type="button" aria-haspopup="dialog" @click="emit('act', 'choose-cover')">
            <Squares2X2Icon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("coverChoice.title") }}</span>
          </button>
        </li>
        <li v-if="offersPhoto">
          <CoverPhotoButton
            pick-only
            look="row"
            class="suggest-photo"
            :work-id="book.id"
            :edition-id="book.edition_id"
            :has-cover="hasCover"
            @picked="(file) => emit('picked', 'photo', file)"
          >
            <template #icon><CameraIcon class="sheet-icon" aria-hidden="true" /></template>
          </CoverPhotoButton>
        </li>
        <li v-if="offersCoverReport">
          <button class="sheet-row report-cover" type="button" @click="emit('act', 'report-cover')">
            <FlagIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("cover.report") }}</span>
          </button>
        </li>
      </ul>
    </section>

    <section v-if="admin" class="actions-group" :aria-labelledby="`${id}-admin`">
      <h3 :id="`${id}-admin`" class="group-head">{{ t("book.groupAdmin") }}</h3>
      <ul class="sheet-rows">
        <li>
          <button class="sheet-row" type="button" @click="emit('act', 'admin-edit')">
            <WrenchScrewdriverIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("admin.editAsAdmin") }}</span>
          </button>
        </li>
        <li v-if="book.edition_id != null">
          <button class="sheet-row" type="button" @click="emit('act', 'admin-edition')">
            <WrenchScrewdriverIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("admin.editEditionAsAdmin") }}</span>
          </button>
        </li>
        <li>
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
          >
            <template #icon><PhotoIcon class="sheet-icon" aria-hidden="true" /></template>
          </CoverPhotoButton>
        </li>
        <li>
          <button class="sheet-row admin-set-edition-cover" type="button" @click="emit('act', 'admin-edition-cover')">
            <PhotoIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("admin.setEditionCover") }}</span>
          </button>
        </li>
      </ul>
    </section>
  </BottomSheet>
</template>

<style scoped>
.actions-group + .actions-group {
  margin-top: var(--space-4);
}

/* A plain sentence-case head over each group, not a caps kicker. */
.group-head {
  margin: 0 0 var(--space-1);
  padding: 0 var(--space-1);
  font-family: var(--sans);
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--text-subtle);
}

.actions-group > .fine {
  margin: 0 0 var(--space-1);
  padding: 0 var(--space-1);
}

.row-end {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}
</style>
