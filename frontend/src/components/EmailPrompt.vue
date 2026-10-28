<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";

/**
 * The one-time prompt for accounts without an email address (imported from
 * the old app): add one, or dismiss the prompt for good (`needs_email_prompt`).
 */
const { t } = useI18n();
const session = useSession();
const toast = useToast();

const open = ref(false);
const email = ref("");
const password = ref("");
const pending = ref(false);
const error = ref("");

async function save() {
  error.value = "";
  pending.value = true;
  try {
    const address = email.value.trim();
    await api.changeEmail(address, password.value);
    session.user = await api.me();
    toast.show(t("emailPrompt.sent", { email: address }));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("emailPrompt.failed");
  } finally {
    pending.value = false;
  }
}

async function dismiss() {
  try {
    await api.dismissEmailPrompt();
  } catch {
    /* It shows again next time; nothing is lost. */
  }
  if (session.user) session.user = { ...session.user, needs_email_prompt: false };
}
</script>

<template>
  <section v-if="session.user?.needs_email_prompt" class="panel email-prompt" aria-labelledby="email-prompt">
    <h2 id="email-prompt">{{ t("emailPrompt.title") }}</h2>
    <p class="fine muted">{{ t("emailPrompt.body") }}</p>
    <form v-if="open" class="email-prompt-form" @submit.prevent="save">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("auth.email") }}</span>
        <input v-model="email" type="email" autocomplete="email" spellcheck="false" required />
      </label>
      <label class="field">
        <span>{{ t("settings.currentPassword") }}</span>
        <input v-model="password" type="password" autocomplete="current-password" required />
      </label>
      <div class="btn-row">
        <button class="btn btn-primary btn-sm" type="submit" :disabled="pending">{{ t("emailPrompt.save") }}</button>
        <button class="btn btn-ghost btn-sm" type="button" @click="open = false">{{ t("common.cancel") }}</button>
      </div>
    </form>
    <div v-else class="btn-row">
      <button class="btn btn-primary btn-sm" type="button" @click="open = true">{{ t("emailPrompt.add") }}</button>
      <button class="btn btn-ghost btn-sm" type="button" @click="dismiss">{{ t("emailPrompt.dismiss") }}</button>
    </div>
  </section>
</template>

<style scoped>
.email-prompt {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-5);
  border-color: var(--accent-line);
}

.email-prompt h2 {
  font-size: var(--text-lg);
}

.email-prompt-form {
  max-width: 420px;
}
</style>
