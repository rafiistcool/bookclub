<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { clearJoinCode, parseInviteCode, rememberJoinCode } from "../invites";
import { useClubs } from "../stores/clubs";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { InvitePreview } from "../types";

/**
 * An invite link (`/join/<code>`). Signed in, one tap joins the club. Signed
 * out, the code waits (in the login `next` and in this tab) while the reader
 * signs in or creates an account, and then they come back here.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const service = useService();
const session = useSession();
const clubs = useClubs();
const toast = useToast();

const code = computed(() => parseInviteCode(String(route.params.code ?? "")));
const preview = ref<InvitePreview | null>(null);
const loaded = ref(false);
const error = ref("");
const pending = ref(false);

const signedIn = computed(() => Boolean(session.user));
const club = computed(() => preview.value?.club ?? null);
const alreadyIn = computed(() =>
  Boolean(club.value && session.user?.clubs.some((row) => row.slug === club.value!.slug)),
);
const loginLink = computed(() => ({ path: "/login", query: { next: `/join/${code.value}` } }));
const signupLink = computed(() => ({ path: "/register", query: { code: code.value } }));

async function load() {
  try {
    preview.value = await api.previewInvite(code.value);
    error.value = "";
    if (!session.user) rememberJoinCode(code.value);
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("join.loadFailed");
  } finally {
    loaded.value = true;
  }
}

async function join() {
  if (pending.value) return;
  pending.value = true;
  error.value = "";
  try {
    const joined = await api.acceptInvite(code.value);
    clearJoinCode();
    clubs.upsert(joined);
    clubs.remember(joined.slug);
    toast.show(t("join.joined", { club: joined.name }));
    await router.replace(`/c/${joined.slug}`);
  } catch (err) {
    // Already a member: the invite was not used, and the club is right there.
    if (err instanceof ApiError && err.status === 409 && club.value && err.detail === "You are already in this club") {
      clearJoinCode();
      await router.replace(`/c/${club.value.slug}`);
      return;
    }
    error.value = err instanceof ApiError ? err.message : t("join.failed");
  } finally {
    pending.value = false;
  }
}

onMounted(load);
</script>

<template>
  <main class="auth-page join-page">
    <h1 class="wordmark">{{ service.name }}</h1>

    <div v-if="!loaded" class="skeleton join-skeleton" aria-hidden="true" />

    <template v-else-if="preview && preview.kind === 'club' && club">
      <p class="kicker">{{ t("join.kicker") }}</p>
      <h2 class="display join-club">{{ club.name }}</h2>
      <p v-if="error" class="error">{{ error }}</p>

      <template v-if="signedIn">
        <p v-if="alreadyIn" class="lede">{{ t("join.alreadyIn") }}</p>
        <p v-else class="lede">{{ t("join.lede", { name: session.user?.username ?? "" }) }}</p>
        <div class="stack">
          <RouterLink v-if="alreadyIn" class="btn btn-primary btn-block" :to="`/c/${club.slug}`">
            {{ t("join.open") }}
          </RouterLink>
          <button v-else class="btn btn-primary btn-block" type="button" :disabled="pending" @click="join">
            {{ pending ? t("join.joining") : t("join.join", { club: club.name }) }}
          </button>
          <RouterLink class="btn btn-ghost btn-block" to="/">{{ t("join.notNow") }}</RouterLink>
        </div>
      </template>

      <template v-else>
        <p class="lede">{{ t("join.signedOutLede") }}</p>
        <div class="stack">
          <RouterLink class="btn btn-primary btn-block" :to="signupLink">{{ t("join.signUp") }}</RouterLink>
          <RouterLink class="btn btn-ghost btn-block" :to="loginLink">{{ t("join.signIn") }}</RouterLink>
        </div>
      </template>
    </template>

    <!-- A sign-up invite: it unlocks an account, not a club. -->
    <template v-else-if="preview">
      <p class="lede">{{ signedIn ? t("join.signupInviteSignedIn") : t("join.signupInvite") }}</p>
      <div class="stack">
        <RouterLink v-if="!signedIn" class="btn btn-primary btn-block" :to="signupLink">{{ t("join.signUp") }}</RouterLink>
        <RouterLink v-else class="btn btn-primary btn-block" to="/">{{ t("join.home") }}</RouterLink>
      </div>
    </template>

    <template v-else>
      <p class="error">{{ error }}</p>
      <p class="fine muted">{{ t("join.invalidHint") }}</p>
      <div class="stack">
        <RouterLink class="btn btn-ghost btn-block" :to="signedIn ? '/clubs' : '/login'">
          {{ signedIn ? t("join.toClubs") : t("join.signIn") }}
        </RouterLink>
      </div>
    </template>

    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
  </main>
</template>

<style scoped>
.join-skeleton {
  height: 160px;
  margin-top: var(--space-5);
}

.join-page .kicker {
  margin-top: var(--space-5);
}

.join-club {
  margin: var(--space-1) 0 var(--space-3);
  overflow-wrap: anywhere;
}

.join-page .lede {
  margin-bottom: var(--space-5);
}

.join-page .error {
  margin-top: var(--space-5);
}

.auth-locale {
  margin-top: var(--space-5);
}
</style>
