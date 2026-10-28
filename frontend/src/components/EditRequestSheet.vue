<script setup lang="ts">
import { vAutosize } from "../autosize";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { BookDetailOut, EditionOut } from "../api/generated";
import { EDITION_FORMATS, editionLine } from "../editions";
import {
  changedFields,
  EDITION_FIELDS,
  editionDraft,
  WORK_FIELDS,
  workDraft,
  type Draft,
  type FieldDef,
} from "../editRequests";
import { languageOptions } from "../languages";
import { useLocale } from "../stores/locale";
import { useToast } from "../stores/toast";
import BottomSheet from "./BottomSheet.vue";

/**
 * "Suggest a correction": the catalog's facts of a book, or of one edition,
 * as a form. Only the changed fields go to the admin queue; an admin applies
 * them, and no catalog source overwrites them afterwards.
 *
 * With `admin` (an instance admin's "Edit as admin"), the same form writes the
 * changed fields at once (`PATCH /api/admin/works/…` or `/editions/…`) and
 * emits `saved` with the book or edition as it is now.
 */
const props = defineProps<{ book?: BookDetailOut; edition?: EditionOut; admin?: boolean }>();
const emit = defineEmits<{ close: []; saved: [value: BookDetailOut | EditionOut] }>();

const { t } = useI18n();
const locale = useLocale();
const toast = useToast();

const defs = computed<readonly FieldDef[]>(() => (props.edition ? EDITION_FIELDS : WORK_FIELDS));
const before: Draft = props.edition ? editionDraft(props.edition) : props.book ? workDraft(props.book) : {};
const draft = ref<Draft>({ ...before });
const note = ref("");
const busy = ref(false);
const error = ref("");
const invalid = ref<string[]>([]);

const languages = computed(() => languageOptions(locale.locale, before.language ? String(before.language) : null));
const subject = computed(() => (props.edition ? editionLine(props.edition) : props.book?.title ?? ""));

function label(def: FieldDef): string {
  return t(`editRequest.fields.${def.key}`);
}

async function send() {
  if (busy.value) return;
  const changes = changedFields(defs.value, before, draft.value);
  invalid.value = changes.invalid;
  if (changes.invalid.length) {
    error.value = t("editRequest.invalid", { fields: changes.invalid.map((key) => t(`editRequest.fields.${key}`)).join(", ") });
    return;
  }
  if (Object.keys(changes.fields).length === 0) {
    error.value = t("editRequest.nothingChanged");
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    if (props.admin) {
      await save(changes.fields);
    } else {
      if (props.edition) {
        await api.requestEditionEdit(props.edition.id, changes.fields, note.value.trim());
      } else if (props.book) {
        await api.requestWorkEdit(props.book.id, changes.fields, note.value.trim());
      }
      toast.show(t("editRequest.sent"));
    }
    emit("close");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t(props.admin ? "admin.edit.failed" : "editRequest.failed");
  } finally {
    busy.value = false;
  }
}

/** The admin's values, written at once. */
async function save(fields: Parameters<typeof api.adminEditWork>[1]) {
  if (props.edition) {
    emit("saved", await api.adminEditEdition(props.edition.id, fields));
  } else if (props.book) {
    emit("saved", await api.adminEditWork(props.book.id, fields));
  }
  toast.show(t("admin.edit.saved"));
}

const title = computed(() => {
  if (props.admin) return props.edition ? t("admin.edit.editionTitle") : t("admin.edit.title");
  return props.edition ? t("editRequest.editionTitle") : t("editRequest.title");
});
</script>

<template>
  <BottomSheet :title="title" @close="emit('close')">
    <p class="fine muted">{{ admin ? t("admin.edit.blurb", { title: subject }) : t("editRequest.blurb", { title: subject }) }}</p>
    <form class="stack edit-request" @submit.prevent="send">
      <label v-for="def in defs" :key="def.key" class="field" :class="{ invalid: invalid.includes(def.key) }">
        <span>{{ label(def) }}</span>
        <textarea
          v-if="def.kind === 'textarea' || def.kind === 'subjects'"
          v-model="draft[def.key]"
          v-autosize
          :name="def.key"
          :rows="def.kind === 'subjects' ? 3 : 4"
          :maxlength="def.kind === 'textarea' ? def.maxlength : undefined"
          :disabled="busy"
        />
        <select v-else-if="def.kind === 'format'" v-model="draft[def.key]" :name="def.key" :disabled="busy">
          <option value="">{{ t("editions.noFormat") }}</option>
          <option v-for="format in EDITION_FORMATS" :key="format" :value="format">{{ t(`editions.format.${format}`) }}</option>
        </select>
        <select v-else-if="def.kind === 'language'" v-model="draft[def.key]" :name="def.key" :disabled="busy">
          <option value="">{{ t("editions.noLanguage") }}</option>
          <option v-for="language in languages" :key="language.code" :value="language.code">{{ language.name }}</option>
        </select>
        <input
          v-else-if="def.kind === 'number'"
          v-model="draft[def.key]"
          :name="def.key"
          type="number"
          inputmode="numeric"
          step="1"
          :min="def.min"
          :max="def.max"
          :disabled="busy"
        />
        <input
          v-else
          v-model="draft[def.key]"
          :name="def.key"
          type="text"
          autocomplete="off"
          :maxlength="def.maxlength"
          :required="def.required"
          :disabled="busy"
        />
        <span v-if="def.kind === 'subjects'" class="field-hint">{{ t("editRequest.subjectsHint") }}</span>
      </label>
      <label v-if="!admin" class="field">
        <span>{{ t("editRequest.note") }}</span>
        <textarea v-model="note" name="note" rows="2" maxlength="2000" :placeholder="t('common.optional')" :disabled="busy" />
      </label>
      <p v-if="error" class="error">{{ error }}</p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy">
        {{ admin ? t("admin.edit.save") : t("editRequest.send") }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.edit-request {
  margin-top: var(--space-3);
}

.field.invalid input,
.field.invalid textarea {
  border-color: var(--danger);
}
</style>
