<script lang="ts">
import { api, ApiError } from "../api/client";
import type { BookOut } from "../api/generated";
import { i18n } from "../i18n";
import { useCoverScan } from "../stores/coverScan";
import { useToast } from "../stores/toast";

const MAX_BYTES = 50 * 1024 * 1024;

/** Where a photo goes: a reader's photo of their copy, or an instance admin's direct cover. */
export interface CoverTarget {
  workId: number;
  editionId: number | null;
  admin?: boolean;
}

/**
 * Straighten a chosen picture in the cover scanner, upload what the reader
 * kept and say how it went in a toast. Resolves to the book when the picture
 * is its cover now, and to null when the reader cancelled the scan, or it
 * waits for an admin, is too large, or failed. The book page calls it too,
 * for a file chosen in a sheet that closed as soon as the file was picked.
 */
export async function sendCoverPhoto(target: CoverTarget, chosen: File): Promise<BookOut | null> {
  const t = i18n.global.t;
  const toast = useToast();
  const file = await useCoverScan().request(chosen);
  if (!file) return null;
  if (file.size > MAX_BYTES) {
    toast.show(t("errors.pictureTooLarge"));
    return null;
  }
  try {
    if (target.admin) {
      const answer = await api.adminSetCover(target.workId, file, target.editionId);
      toast.show(t("admin.coverSet"));
      return answer.book;
    }
    const answer = await api.uploadCover(target.workId, file, target.editionId);
    if ("book" in answer) {
      toast.show(t("cover.photoSet"));
      return answer.book;
    }
    toast.show(t("cover.photoQueued"));
    return null;
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("cover.photoFailed"));
    return null;
  }
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

/**
 * "Add a photo of your copy" under the book page's cover. Without a cover the
 * photo becomes the cover at once (`covered`); with one it waits for an admin.
 * The server strips the metadata and resizes it.
 *
 * `admin` is an instance admin's direct cover (#74): the picture replaces any
 * cover of the book, or of `editionId`, at once. `label` and `look` let the
 * cover prompt and the admin pages show it as a button or an inline link.
 *
 * `pickOnly` uploads nothing: it hands the chosen file on (`picked`), for a
 * sheet that closes at once and leaves the upload to its page (an unmounted
 * component's emits are dropped, so the sheet could not report the cover).
 * The tap still lands on the real file input, which iOS needs for the picker.
 */
const props = withDefaults(
  defineProps<{
    workId: number;
    editionId: number | null;
    hasCover: boolean;
    /** The text instead of "Add/Suggest a photo of your copy". */
    label?: string;
    /** `fine`: the small link under a cover; `inline`: a link in running text; `primary`/`ghost`: a small button; `row`: a light row of a sheet (`.sheet-row`, with the `icon` slot). */
    look?: "fine" | "inline" | "primary" | "ghost" | "row";
    admin?: boolean;
    pickOnly?: boolean;
  }>(),
  { label: undefined, look: "fine", admin: false, pickOnly: false },
);
const emit = defineEmits<{ covered: [book: BookOut]; picked: [file: File] }>();

const LOOKS = {
  fine: "text-btn finer",
  inline: "text-btn",
  primary: "btn btn-primary btn-sm",
  ghost: "btn btn-ghost btn-sm",
  row: "sheet-row",
} as const;

const { t } = useI18n();
const busy = ref(false);

const text = computed(() => {
  if (busy.value) return t("cover.photoUploading");
  if (props.label) return props.label;
  return props.hasCover ? t("cover.photoSuggest") : t("cover.photoAdd");
});

async function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file || busy.value) return;
  if (props.pickOnly) {
    emit("picked", file);
    return;
  }
  busy.value = true;
  try {
    const book = await sendCoverPhoto({ workId: props.workId, editionId: props.editionId, admin: props.admin }, file);
    if (book) emit("covered", book);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <label class="cover-photo" :class="[LOOKS[look], { busy }]">
    <input
      type="file"
      accept="image/jpeg,image/png,image/webp,image/gif"
      :disabled="busy"
      @change="onFile"
    />
    <template v-if="look === 'row'">
      <slot name="icon" />
      <span class="sheet-label">{{ text }}</span>
    </template>
    <template v-else>{{ text }}</template>
  </label>
</template>

<style scoped>
.cover-photo {
  position: relative;
  cursor: pointer;
}

.cover-photo input {
  position: absolute;
  inset: 0;
  width: 100%;
  opacity: 0;
  cursor: pointer;
}

/* The input is invisible, so its keyboard focus shows on the label. */
.cover-photo:has(input:focus-visible) {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.cover-photo.busy {
  opacity: 0.6;
}
</style>
