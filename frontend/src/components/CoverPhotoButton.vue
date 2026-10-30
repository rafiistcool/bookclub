<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { BookOut } from "../api/generated";
import { useToast } from "../stores/toast";

/**
 * "Add a photo of your copy" under the book page's cover. Without a cover the
 * photo becomes the cover at once (`covered`); with one it waits for an admin.
 * The server strips the metadata and resizes it.
 *
 * `admin` is an instance admin's direct cover (#74): the picture replaces any
 * cover of the book, or of `editionId`, at once. `label` and `look` let the
 * cover prompt and the admin pages show it as a button or an inline link.
 */
const props = withDefaults(
  defineProps<{
    workId: number;
    editionId: number | null;
    hasCover: boolean;
    /** The text instead of "Add/Suggest a photo of your copy". */
    label?: string;
    /** `fine`: the small link under a cover; `inline`: a link in running text; `primary`/`ghost`: a small button. */
    look?: "fine" | "inline" | "primary" | "ghost";
    admin?: boolean;
  }>(),
  { label: undefined, look: "fine", admin: false },
);
const emit = defineEmits<{ covered: [book: BookOut] }>();

const MAX_BYTES = 50 * 1024 * 1024;
const LOOKS = {
  fine: "text-btn finer",
  inline: "text-btn",
  primary: "btn btn-primary btn-sm",
  ghost: "btn btn-ghost btn-sm",
} as const;

const { t } = useI18n();
const toast = useToast();
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
  if (file.size > MAX_BYTES) {
    toast.show(t("errors.pictureTooLarge"));
    return;
  }
  busy.value = true;
  try {
    if (props.admin) {
      const answer = await api.adminSetCover(props.workId, file, props.editionId);
      emit("covered", answer.book);
      toast.show(t("admin.coverSet"));
      return;
    }
    const answer = await api.uploadCover(props.workId, file, props.editionId);
    if ("book" in answer) {
      emit("covered", answer.book);
      toast.show(t("cover.photoSet"));
    } else {
      toast.show(t("cover.photoQueued"));
    }
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("cover.photoFailed"));
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
    {{ text }}
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
