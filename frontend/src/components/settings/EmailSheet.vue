<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BottomSheet from "../BottomSheet.vue";

/**
 * A new (or a first) email address, confirmed with the current password. The
 * old address stays until the new one is confirmed, and the server answers
 * the same whether or not the address is taken.
 */
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const session = useSession();
const toast = useToast();

const address = ref("");
const password = ref("");
const busy = ref(false);
const error = ref("");

const title = computed(() => (session.user?.email ? t("account.changeEmail") : t("account.addEmail")));

async function save() {
  error.value = "";
  busy.value = true;
  try {
    const next = address.value.trim();
    await api.changeEmail(next, password.value);
    session.user = await api.me();
    toast.show(t("account.emailSent", { email: next }));
    emit("close");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("account.emailFailed");
  } finally {
    busy.value = false;
  }
}

function close() {
  if (!busy.value) emit("close");
}
</script>

<template>
  <BottomSheet class="email-sheet" :title="title" @close="close">
    <form class="sheet-form" @submit.prevent="save">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("account.newEmail") }}</span>
        <input v-model="address" type="email" autocomplete="email" spellcheck="false" required />
      </label>
      <label class="field">
        <span>{{ t("settings.currentPassword") }}</span>
        <input v-model="password" type="password" autocomplete="current-password" required />
      </label>
      <p v-if="session.user?.email" class="field-hint">{{ t("account.emailHint") }}</p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy">{{ t("account.saveEmail") }}</button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.sheet-form {
  display: grid;
  gap: var(--space-3);
}
</style>
