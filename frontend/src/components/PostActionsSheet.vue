<script setup lang="ts">
import { PencilSquareIcon, TrashIcon } from "@heroicons/vue/24/outline";
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import BottomSheet from "./BottomSheet.vue";

/**
 * A post's "…" sheet: what is not reacting or replying, as light rows like
 * the book page's "…" sheet. The writer edits and deletes their own post; a
 * club admin deletes someone else's club post (moderation), and only here,
 * never as a link beside "Reply". Deleting asks once more in place: the row
 * turns into "Really delete?", and a second tap deletes. The sheet only says
 * what was asked (`act`); the post closes it and does the rest.
 */
const props = defineProps<{
  title: string;
  canEdit: boolean;
  canDelete: boolean;
  /** Deleting someone else's post as a club admin. */
  moderating?: boolean;
}>();
const emit = defineEmits<{ close: []; act: [action: "edit" | "delete"] }>();

const { t } = useI18n();
const confirming = ref(false);

function remove() {
  if (!confirming.value) {
    confirming.value = true;
    return;
  }
  emit("act", "delete");
}
</script>

<template>
  <BottomSheet class="post-actions" :title="props.title" @close="emit('close')">
    <ul class="sheet-rows">
      <li v-if="canEdit">
        <button class="sheet-row post-edit" type="button" @click="emit('act', 'edit')">
          <PencilSquareIcon class="sheet-icon" aria-hidden="true" />
          <span class="sheet-label">{{ t("common.edit") }}</span>
        </button>
      </li>
      <li v-if="canDelete">
        <button class="sheet-row post-delete" :class="{ danger: confirming }" type="button" @click="remove">
          <TrashIcon class="sheet-icon" aria-hidden="true" />
          <span class="sheet-label" aria-live="polite">
            {{ confirming ? t("diary.deleteConfirm") : moderating ? t("diary.moderate") : t("common.delete") }}
          </span>
        </button>
      </li>
    </ul>
  </BottomSheet>
</template>
