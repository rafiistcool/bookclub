<script setup lang="ts">
import { vAutosize } from "../autosize";
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { useToast } from "../stores/toast";
import BottomSheet from "./BottomSheet.vue";

const { t } = useI18n();

const props = defineProps<{
  initialTitle?: string;
}>();

const emit = defineEmits<{ close: [] }>();

const router = useRouter();
const toast = useToast();
const title = ref(props.initialTitle?.trim() || "");
const authors = ref("");
const year = ref("");
const isbn = ref("");
const description = ref("");
const busy = ref(false);
const error = ref("");
/** A 409 for a known ISBN names the book the catalog already has. */
const existingWorkId = ref<number | null>(null);

async function open(path: string) {
  emit("close");
  await router.push(path);
}

async function submit() {
  if (busy.value) return;
  const trimmed = title.value.trim();
  if (!trimmed) {
    error.value = t("addBook.titleRequired");
    return;
  }
  busy.value = true;
  error.value = "";
  existingWorkId.value = null;
  try {
    const yearRaw = String(year.value ?? "").trim();
    const parsedYear = yearRaw ? Number(yearRaw) : null;
    const book = await api.createCustomBook({
      title: trimmed,
      authors: authors.value.trim(),
      year: parsedYear && Number.isFinite(parsedYear) ? parsedYear : null,
      description: description.value.trim(),
      ...(isbn.value.trim() ? { isbn: isbn.value.trim() } : {}),
    });
    toast.show(t("addBook.added"));
    await open(bookPath(book));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("addBook.failed");
    if (err instanceof ApiError && err.status === 409) {
      const workId = (err.body as { work_id?: unknown } | null)?.work_id;
      existingWorkId.value = typeof workId === "number" ? workId : null;
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <BottomSheet :title="t('addBook.title')" @close="emit('close')">
    <p class="fine subtle lede">
      {{ t("addBook.lede") }}
    </p>
    <form class="stack" @submit.prevent="submit">
      <label class="field">
        <span>{{ t("addBook.bookTitle") }}</span>
        <input v-model="title" type="text" maxlength="500" required autocomplete="off" />
      </label>
      <label class="field">
        <span>{{ t("addBook.author") }}</span>
        <input v-model="authors" type="text" maxlength="300" autocomplete="off" />
      </label>
      <label class="field">
        <span>{{ t("addBook.year") }}</span>
        <input v-model="year" type="number" min="1" max="3000" inputmode="numeric" />
      </label>
      <label class="field">
        <span>{{ t("addBook.isbn") }}</span>
        <input v-model="isbn" type="text" maxlength="20" inputmode="numeric" autocomplete="off" spellcheck="false" />
        <span class="field-hint">{{ t("addBook.isbnHint") }}</span>
      </label>
      <label class="field">
        <span>{{ t("addBook.notes") }}</span>
        <textarea v-autosize v-model="description" rows="3" maxlength="4000" />
      </label>
      <p v-if="error" class="fine danger">
        {{ error }}
        <button
          v-if="existingWorkId"
          class="text-btn"
          type="button"
          @click="open(bookPath(existingWorkId))"
        >
          {{ t("addBook.openExisting") }}
        </button>
      </p>
      <div class="btn-row">
        <button class="btn btn-primary" type="submit" :disabled="busy">
          {{ busy ? t("addBook.adding") : t("addBook.add") }}
        </button>
        <button class="btn btn-ghost" type="button" :disabled="busy" @click="emit('close')">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>
  </BottomSheet>
</template>

<style scoped>
.lede {
  margin-bottom: var(--space-4);
}

.stack {
  display: grid;
  gap: var(--space-3);
}

.danger {
  color: var(--danger, #b42318);
}
</style>
