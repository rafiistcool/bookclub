<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import BottomSheet from "./BottomSheet.vue";

/** Notice-and-takedown: tell an admin that a cover is wrong, offensive or infringes copyright. */
const props = defineProps<{ publicId: string; title: string }>();
const emit = defineEmits<{ close: [] }>();

const REASONS = ["wrong_book", "offensive", "copyright", "other"] as const;

const { t } = useI18n();
const toast = useToast();
const reason = ref<(typeof REASONS)[number] | "">("");
const details = ref("");
const busy = ref(false);
const error = ref("");

async function submit() {
  if (!reason.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    const created = await api.reportCover(props.publicId, { reason: reason.value, details: details.value.trim() || null });
    // A repeat names the reader's open report; a picture taken down meanwhile needs nobody.
    toast.show(created.status === "removed" ? t("cover.alreadyRemoved") : t("cover.reported"));
    emit("close");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("cover.reportFailed");
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <BottomSheet :title="t('cover.reportTitle')" @close="emit('close')">
    <p class="muted report-book">{{ title }}</p>
    <form class="stack" @submit.prevent="submit">
      <fieldset class="report-reasons">
        <legend class="fine">{{ t("cover.reportWhy") }}</legend>
        <label v-for="option in REASONS" :key="option" class="report-reason">
          <input v-model="reason" type="radio" name="reason" :value="option" :disabled="busy" />
          {{ t(`cover.reasons.${option}`) }}
        </label>
      </fieldset>
      <label class="field">
        <span>{{ t("cover.reportDetails") }}</span>
        <textarea v-model="details" rows="3" maxlength="2000" :placeholder="t('common.optional')" :disabled="busy" />
      </label>
      <p v-if="error" class="error">{{ error }}</p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy || !reason">
        {{ busy ? t("cover.reporting") : t("cover.reportSend") }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.report-book { margin-bottom: var(--space-2); }
.report-reasons { display: grid; gap: var(--space-2); margin: 0; padding: 0; border: 0; }
.report-reasons legend { margin-bottom: var(--space-2); }
.report-reason { display: flex; align-items: center; gap: var(--space-2); min-height: var(--tap); }
</style>
