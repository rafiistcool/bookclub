<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import type { VerifyEmailPreview } from "../api/generated";
import { linkToken } from "../authToken";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

/**
 * The verification mail's link (`/verify-email#token=…`). It works on any
 * device, signed in or not, and signs nobody in. The token leaves the address
 * bar as soon as it is read.
 *
 * Opening the page verifies nothing: it names the account and the address,
 * and only the reader's tap verifies. A mail scanner that opens links, or a
 * click on a mail someone else's sign-up sent, must never hand the address to
 * an account its owner does not know.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const service = useService();
const session = useSession();

const state = ref<"loading" | "confirm" | "working" | "done" | "failed">("loading");
const preview = ref<VerifyEmailPreview | null>(null);
const error = ref("");
const signedIn = computed(() => Boolean(session.user));
const finished = computed(() => state.value === "done" || state.value === "failed");
let token = "";

function fail(err: unknown, fallback: string) {
  state.value = "failed";
  error.value = err instanceof ApiError ? err.message : fallback;
}

onMounted(async () => {
  token = linkToken(route.hash, route.query.token) ?? "";
  void router.replace({ path: route.path, query: {}, hash: "" });
  if (!token) {
    fail(null, t("errors.linkExpired"));
    return;
  }
  try {
    preview.value = await api.previewVerifyEmail(token);
    state.value = "confirm";
  } catch (err) {
    fail(err, t("verify.failed"));
  }
});

async function confirm() {
  if (state.value !== "confirm") return;
  state.value = "working";
  try {
    await api.verifyEmail(token);
    state.value = "done";
    if (session.user) session.user = await api.me().catch(() => session.user);
  } catch (err) {
    fail(err, t("verify.failed"));
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <p v-if="state === 'loading'" class="lede">{{ t("verify.loading") }}</p>
    <template v-else-if="(state === 'confirm' || state === 'working') && preview">
      <h2 class="auth-title">{{ t("verify.confirmTitle") }}</h2>
      <p class="lede">{{ t("verify.confirmBody", { email: preview.email, username: preview.username }) }}</p>
      <p class="fine muted">{{ t("verify.notMine") }}</p>
      <div class="stack auth-next">
        <button class="btn btn-primary btn-block" type="button" :disabled="state === 'working'" @click="confirm">
          {{ state === "working" ? t("verify.working") : t("verify.confirm") }}
        </button>
      </div>
    </template>
    <template v-else-if="state === 'done'">
      <h2 class="auth-title">{{ t("verify.doneTitle") }}</h2>
      <p class="lede">{{ t("verify.doneBody") }}</p>
    </template>
    <template v-else>
      <h2 class="auth-title">{{ t("verify.failedTitle") }}</h2>
      <p class="error">{{ error }}</p>
      <p class="fine muted">{{ t("verify.failedHint") }}</p>
    </template>
    <div v-if="finished" class="stack auth-next">
      <RouterLink class="btn btn-primary btn-block" :to="signedIn ? (state === 'done' ? '/' : '/settings') : '/login'">
        {{ signedIn ? (state === "done" ? t("verify.toHome") : t("verify.toSettings")) : t("auth.signIn") }}
      </RouterLink>
    </div>
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

.lede,
.error {
  margin: var(--space-2) 0 var(--space-4);
}

.auth-next,
.auth-locale {
  margin-top: var(--space-5);
}
</style>
