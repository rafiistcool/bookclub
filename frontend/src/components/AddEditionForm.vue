<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { EditionOut, NewEditionIn } from "../api/generated";
import { bookPath } from "../constants";
import { EDITION_FORMATS } from "../editions";
import { languageOptions } from "../languages";
import { useLocale } from "../stores/locale";

/**
 * "My edition is not listed": adds a missing edition of the work
 * (`POST …/editions`). With an ISBN the server looks it up first; an edition
 * it already knows is 409, either of this book (choose it) or of another one
 * (open that book).
 */
const props = defineProps<{ workId: number }>();
const emit = defineEmits<{
  added: [edition: EditionOut];
  /** The ISBN names an edition of this work that is already listed. */
  existing: [editionId: number];
  cancel: [];
}>();

const { t } = useI18n();
const locale = useLocale();

const isbn = ref("");
const format = ref("");
const publisher = ref("");
const year = ref<number | "">("");
const pages = ref<number | "">("");
const language = ref("");
const title = ref("");
const busy = ref(false);
const error = ref("");
/** A 409 for the ISBN: where the catalog already has it. */
const known = ref<{ workId: number; editionId: number | null } | null>(null);

const languages = computed(() => languageOptions(locale.locale));

function text(value: string): string | null {
  return value.trim() || null;
}

function whole(value: number | ""): number | null {
  return value === "" || !Number.isFinite(Number(value)) ? null : Math.trunc(Number(value));
}

async function submit() {
  if (busy.value) return;
  const body: NewEditionIn = {
    isbn: text(isbn.value),
    format: format.value || null,
    publisher: text(publisher.value),
    publication_year: whole(year.value),
    pages: whole(pages.value),
    language: language.value || null,
    title: text(title.value),
  };
  if (Object.values(body).every((value) => value == null)) {
    error.value = t("editions.addSomething");
    return;
  }
  busy.value = true;
  error.value = "";
  known.value = null;
  try {
    emit("added", await api.addEdition(props.workId, body));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("editions.addFailed");
    if (err instanceof ApiError && err.status === 409) {
      const found = err.body as { work_id?: unknown; edition_id?: unknown } | null;
      if (typeof found?.work_id === "number") {
        known.value = {
          workId: found.work_id,
          editionId: typeof found.edition_id === "number" ? found.edition_id : null,
        };
      }
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <form class="stack add-edition" @submit.prevent="submit">
    <p class="fine muted">{{ t("editions.addBlurb") }}</p>
    <label class="field">
      <span>{{ t("addBook.isbn") }}</span>
      <input v-model="isbn" name="isbn" type="text" maxlength="20" inputmode="numeric" autocomplete="off" spellcheck="false" :disabled="busy" />
      <span class="field-hint">{{ t("editions.isbnHint") }}</span>
    </label>
    <div class="field-pair">
      <label class="field">
        <span>{{ t("editRequest.fields.format") }}</span>
        <select v-model="format" name="format" :disabled="busy">
          <option value="">{{ t("editions.noFormat") }}</option>
          <option v-for="option in EDITION_FORMATS" :key="option" :value="option">{{ t(`editions.format.${option}`) }}</option>
        </select>
      </label>
      <label class="field">
        <span>{{ t("editRequest.fields.language") }}</span>
        <select v-model="language" name="language" :disabled="busy">
          <option value="">{{ t("editions.noLanguage") }}</option>
          <option v-for="option in languages" :key="option.code" :value="option.code">{{ option.name }}</option>
        </select>
      </label>
    </div>
    <label class="field">
      <span>{{ t("editRequest.fields.publisher") }}</span>
      <input v-model="publisher" name="publisher" type="text" maxlength="300" autocomplete="off" :disabled="busy" />
    </label>
    <div class="field-pair">
      <label class="field">
        <span>{{ t("editRequest.fields.publication_year") }}</span>
        <input v-model.number="year" name="year" type="number" min="1" max="3000" inputmode="numeric" :disabled="busy" />
      </label>
      <label class="field">
        <span>{{ t("editRequest.fields.pages") }}</span>
        <input v-model.number="pages" name="pages" type="number" min="1" max="100000" inputmode="numeric" :disabled="busy" />
      </label>
    </div>
    <label class="field">
      <span>{{ t("editions.ownTitle") }}</span>
      <input v-model="title" name="title" type="text" maxlength="1000" autocomplete="off" :disabled="busy" />
      <span class="field-hint">{{ t("editions.ownTitleHint") }}</span>
    </label>
    <p v-if="error" class="error">
      {{ error }}
      <template v-if="known">
        <button
          v-if="known.workId === workId && known.editionId != null"
          class="text-btn"
          type="button"
          @click="emit('existing', known.editionId)"
        >
          {{ t("editions.chooseExisting") }}
        </button>
        <RouterLink v-else-if="known.workId !== workId" :to="bookPath(known.workId)">
          {{ t("addBook.openExisting") }}
        </RouterLink>
      </template>
    </p>
    <div class="btn-row">
      <button class="btn btn-primary btn-sm" type="submit" :disabled="busy">
        {{ busy ? t("editions.adding") : t("editions.add") }}
      </button>
      <button class="btn btn-ghost btn-sm" type="button" :disabled="busy" @click="emit('cancel')">
        {{ t("common.cancel") }}
      </button>
    </div>
  </form>
</template>

<style scoped>
.add-edition {
  margin-top: var(--space-3);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

.field-pair {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
  gap: var(--space-3);
}
</style>
