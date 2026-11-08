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
/** What to do next, under the error: depends on why the sign-in failed. */
const nextStep = ref<"" | "check" | "connection">("");
const pending = ref(false);
/** Coming from an invite link: sign-up keeps the code. */
const signupLink = computed(() => {
  const code = joinCodeFromNext(route.query.next) ?? pendingJoinCode();
  return code ? { path: "/signup", query: { code } } : "/signup";
});

async function submit() {
  error.value = "";
  nextStep.value = "";
  pending.value = true;
  try {
    await session.login(username.value, password.value);
    await router.replace(safeNextPath(route.query.next));
  } catch (err) {
    // A wrong name or password asks for another look (or a new password);
    // no answer at all asks for the connection. A pause or a suspension
    // already says what to do in its own words.
    if (err instanceof ApiError) {
      error.value = err.message;
      nextStep.value = err.status === 401 ? "check" : "";
    } else {
      error.value = t("auth.couldNotSignIn");
      nextStep.value = "connection";
    }
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <form @submit.prevent="submit">
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
      <!-- Under the button, so the fields stay where the fingers are. -->
      <div class="login-error" role="alert">
        <template v-if="error">
          <p class="error">{{ error }}</p>
          <!-- With mail, the way to a new password is the link right below. -->
          <p v-if="nextStep === 'check'" class="fine muted">
            {{ service.mail ? t("auth.checkOrReset") : t("auth.checkOrAskAdmin") }}
          </p>
          <p v-else-if="nextStep === 'connection'" class="fine muted">{{ t("auth.checkConnection") }}</p>
        </template>
      </div>
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
    <p v-if="service.contact_email" class="fine muted auth-contact">
      <i18n-t keypath="report.noAccount" tag="span" scope="global">
        <template #email>
          <a :href="`mailto:${service.contact_email}`">{{ service.contact_email }}</a>
        </template>
      </i18n-t>
    </p>
  </main>
</template>

<style scoped>
form {
  margin-top: var(--space-6);
}

.login-error:not(:empty) {
  margin-top: var(--space-3);
}

.login-error .error {
  margin: 0 0 var(--space-1);
  font-weight: 600;
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

/* Notice and takedown (#54) for people without an account: the public contact address. */
.auth-contact {
  margin-top: var(--space-3);
}

.auth-contact a {
  overflow-wrap: anywhere;
}
</style>
