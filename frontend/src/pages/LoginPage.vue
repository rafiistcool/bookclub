<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { useClub } from "../stores/club";
import { useSession } from "../stores/session";

const { t } = useI18n();

const club = useClub();
const session = useSession();
const router = useRouter();
const route = useRoute();
const username = ref("");
const password = ref("");
const show = ref(false);
const error = ref("");
const pending = ref(false);

function safeNextPath(raw: unknown): string {
  if (typeof raw !== "string") return "/";
  if (!raw.startsWith("/") || raw.startsWith("//") || raw.includes("\\")) {
    return "/";
  }
  return raw;
}

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
    <h1 class="wordmark">{{ club.name }}</h1>
    <p class="lede">{{ t("auth.signInLede") }}</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("auth.username") }}</span>
        <input v-model="username" name="username" autocomplete="username" required />
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
    <p class="muted fine auth-alt">
      {{ t("auth.haveInvite") }}
      <RouterLink to="/register">{{ t("auth.createAccount") }}</RouterLink>
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
