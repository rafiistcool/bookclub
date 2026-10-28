<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import LegalLinks from "../components/LegalLinks.vue";
import { joinCodeFromNext, pendingJoinCode } from "../invites";
import { safeNextPath } from "../nextPath";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

const { t } = useI18n();

const service = useService();
const session = useSession();
const router = useRouter();
const route = useRoute();
const username = ref("");
const password = ref("");
const show = ref(false);
const error = ref("");
const pending = ref(false);
/** Coming from an invite link: sign-up keeps the code. */
const signupLink = computed(() => {
  const code = joinCodeFromNext(route.query.next) ?? pendingJoinCode();
  return code ? { path: "/signup", query: { code } } : "/signup";
});

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.login(username.value, password.value);
    await router.replace(safeNextPath(route.query.next));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("auth.couldNotSignIn");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <p class="lede">{{ t("auth.signInLede") }}</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("auth.username") }}</span>
        <input v-model="username" name="username" autocomplete="username" required />
        <span class="field-hint">{{ t("auth.usernameOrEmail") }}</span>
      </label>
      <label class="field">
        <span>{{ t("auth.password") }}</span>
        <div class="password-wrap">
          <input
            v-model="password"
            :type="show ? 'text' : 'password'"
            name="password"
            autocomplete="current-password"
            required
          />
          <button class="text-btn" type="button" @click="show = !show">
            {{ show ? t("common.hide") : t("common.show") }}
          </button>
        </div>
      </label>
      <button class="btn btn-primary btn-block" type="submit" :disabled="pending">
        {{ pending ? t("auth.signingIn") : t("auth.signIn") }}
      </button>
    </form>
    <p class="fine auth-forgot">
      <RouterLink to="/forgot-password">{{ t("auth.forgotPassword") }}</RouterLink>
    </p>
    <p class="muted fine auth-alt">
      {{ service.signup === "open" ? t("auth.newHere") : t("auth.haveInvite") }}
      <RouterLink :to="signupLink">{{ t("auth.createAccount") }}</RouterLink>
    </p>
    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
    <LegalLinks class="auth-legal" />
  </main>
</template>

<style scoped>
.lede {
  margin: var(--space-2) 0 var(--space-6);
}

.auth-forgot {
  margin-top: var(--space-3);
}

.auth-alt {
  margin-top: var(--space-5);
}

.auth-locale {
  margin-top: var(--space-5);
}

.auth-legal {
  margin-top: var(--space-4);
}
</style>
