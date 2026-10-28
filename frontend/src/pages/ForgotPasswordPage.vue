<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { useService } from "../stores/service";

/**
 * Asks for a reset link. The answer is the same whether or not the address
 * belongs to an account, so the page cannot tell anyone who has one.
 */
const { t } = useI18n();
const service = useService();

const email = ref("");
const pending = ref(false);
const sent = ref(false);
const error = ref("");

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await api.requestPasswordReset(email.value.trim());
    sent.value = true;
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("forgot.failed");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <template v-if="sent">
      <h2 class="auth-title">{{ t("forgot.sentTitle") }}</h2>
      <p class="lede">{{ t("forgot.sentBody", { email: email.trim() }) }}</p>
    </template>
    <template v-else>
      <p class="lede">{{ t("forgot.lede") }}</p>
      <form @submit.prevent="submit">
        <p v-if="error" class="error">{{ error }}</p>
        <label class="field">
          <span>{{ t("auth.email") }}</span>
          <input v-model="email" type="email" name="email" autocomplete="email" spellcheck="false" required />
        </label>
        <button class="btn btn-primary btn-block" type="submit" :disabled="pending">
          {{ pending ? t("forgot.sending") : t("forgot.send") }}
        </button>
      </form>
      <p class="fine muted forgot-note">{{ t("forgot.noEmailNote") }}</p>
    </template>
    <p class="muted fine auth-alt">
      <RouterLink to="/login">{{ t("forgot.backToLogin") }}</RouterLink>
    </p>
    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
  </main>
</template>

<style scoped>
.auth-title {
  margin-top: var(--space-5);
  font-size: var(--text-xl);
}

.lede {
  margin: var(--space-2) 0 var(--space-6);
}

.forgot-note,
.auth-alt,
.auth-locale {
  margin-top: var(--space-5);
}
</style>
