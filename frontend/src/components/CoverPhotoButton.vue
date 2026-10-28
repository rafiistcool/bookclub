<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { BookOut } from "../api/generated";
import { useToast } from "../stores/toast";

/**
 * "Add a photo of your copy" under the book page's cover. Without a cover the
 * photo becomes the cover at once (`covered`); with one it waits for an admin.
 * The server strips the metadata and resizes it.
 */
const props = defineProps<{ workId: number; editionId: number | null; hasCover: boolean }>();
const emit = defineEmits<{ covered: [book: BookOut] }>();

const MAX_BYTES = 50 * 1024 * 1024;

const { t } = useI18n();
const toast = useToast();
const busy = ref(false);

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
  <label class="text-btn finer cover-photo" :class="{ busy }">
    <input
      type="file"
      accept="image/jpeg,image/png,image/webp,image/gif"
      :disabled="busy"
      @change="onFile"
    />
    {{ busy ? t("cover.photoUploading") : hasCover ? t("cover.photoSuggest") : t("cover.photoAdd") }}
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

.cover-photo.busy {
  opacity: 0.6;
}
</style>
