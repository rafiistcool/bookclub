<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import { linkToken } from "../authToken";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

/**
 * The reset mail's link (`/reset-password#token=…`, valid for an hour). A new
 * password ends every session of the account, this one included, so the
 * reader signs in again afterwards.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const service = useService();
const session = useSession();

const token = ref<string | null>(null);
const password = ref("");
const show = ref(false);
const pending = ref(false);
const done = ref(false);
const error = ref("");

onMounted(() => {
  token.value = linkToken(route.hash, route.query.token);
  void router.replace({ path: route.path, query: {}, hash: "" });
  if (!token.value) error.value = t("errors.linkExpired");
});

async function submit() {
  if (!token.value) return;
  error.value = "";
  pending.value = true;
  try {
    await api.confirmPasswordReset(token.value, password.value);
    done.value = true;
    // The server ended every session; forget the one this tab had.
    if (session.user) session.reset();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("reset.failed");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <template v-if="done">
      <h2 class="auth-title">{{ t("reset.doneTitle") }}</h2>
      <p class="lede">{{ t("reset.doneBody") }}</p>
      <RouterLink class="btn btn-primary btn-block" to="/login">{{ t("auth.signIn") }}</RouterLink>
    </template>
    <template v-else>
      <p class="lede">{{ t("reset.lede") }}</p>
      <p v-if="error" class="error">{{ error }}</p>
      <form v-if="token" @submit.prevent="submit">
        <label class="field">
          <span>{{ t("reset.newPassword") }}</span>
          <div class="password-wrap">
            <input
              v-model="password"
              :type="show ? 'text' : 'password'"
              name="new-password"
              autocomplete="new-password"
              minlength="8"
              required
            />
            <button class="text-btn" type="button" @click="show = !show">
              {{ show ? t("common.hide") : t("common.show") }}
            </button>
          </div>
        </label>
        <button class="btn btn-primary btn-block" type="submit" :disabled="pending">
          {{ pending ? t("reset.saving") : t("reset.save") }}
        </button>
      </form>
      <p v-else class="fine muted">
        <RouterLink to="/forgot-password">{{ t("reset.newLink") }}</RouterLink>
      </p>
    </template>
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

.auth-locale {
  margin-top: var(--space-5);
}
</style>
