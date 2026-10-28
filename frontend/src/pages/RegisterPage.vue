<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

const { t } = useI18n();

const service = useService();

const session = useSession();
const router = useRouter();
const username = ref("");
const email = ref("");
const password = ref("");
const invite = ref("");
const show = ref(false);
const error = ref("");
const pending = ref(false);

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.register({
      username: username.value,
      email: email.value,
      password: password.value,
      inviteCode: invite.value,
    });
    await router.replace("/");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("auth.couldNotCreate");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <p class="lede">{{ t("auth.joinLede") }}</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("auth.inviteCode") }}</span>
        <input
          v-model="invite"
          name="invite"
          autocomplete="one-time-code"
          autocapitalize="characters"
          required
        />
      </label>
      <label class="field">
        <span>{{ t("auth.username") }}</span>
        <input
          v-model="username"
          name="username"
          autocomplete="username"
          spellcheck="false"
          required
        />
      </label>
      <label class="field">
        <span>{{ t("auth.email") }}</span>
        <input
          v-model="email"
          type="email"
          name="email"
          autocomplete="email"
          spellcheck="false"
          required
        />
      </label>
      <label class="field">
        <span>{{ t("auth.password") }}</span>
        <div class="password-wrap">
          <input
            v-model="password"
            :type="show ? 'text' : 'password'"
            name="password"
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
        {{ pending ? t("auth.creating") : t("auth.createAccount") }}
      </button>
    </form>
    <p class="muted fine auth-alt">
      {{ t("auth.alreadyHave") }}
      <RouterLink to="/login">{{ t("auth.signIn") }}</RouterLink>
    </p>
    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
  </main>
</template>

<style scoped>
.lede {
  margin: var(--space-2) 0 var(--space-6);
}

.auth-alt {
  margin-top: var(--space-5);
}

.auth-locale {
  margin-top: var(--space-5);
}
</style>
