<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import { workId } from "../constants";
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
const description = ref("");
const busy = ref(false);
const error = ref("");

async function submit() {
  if (busy.value) return;
  const trimmed = title.value.trim();
  if (!trimmed) {
    error.value = t("addBook.titleRequired");
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    const yearRaw = String(year.value ?? "").trim();
    const parsedYear = yearRaw ? Number(yearRaw) : null;
    const book = await api.createCustomBook({
      title: trimmed,
      authors: authors.value.trim(),
      year: parsedYear && Number.isFinite(parsedYear) ? parsedYear : null,
      description: description.value.trim(),
    });
    toast.show(t("addBook.added"));
    emit("close");
    await router.push(`/book/${workId(book.ol_work_key)}`);
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("addBook.failed");
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
        <span>{{ t("addBook.notes") }}</span>
        <textarea v-model="description" rows="3" maxlength="4000" />
      </label>
      <p v-if="error" class="fine danger">{{ error }}</p>
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
