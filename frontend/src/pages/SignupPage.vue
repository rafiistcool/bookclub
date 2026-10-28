<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "../api/client";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { clearJoinCode, parseInviteCode, pendingJoinCode } from "../invites";
import { useClubs } from "../stores/clubs";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";

/**
 * Sign-up. In `open` mode (`ServiceConfig.signup`) anyone may create an
 * account and an invite code is optional (a club invite also joins the
 * club); in `invite` mode the code is required. The browser's time zone and
 * the UI language go along.
 */
const { t } = useI18n();
const service = useService();
const session = useSession();
const router = useRouter();
const route = useRoute();
const clubs = useClubs();
const toast = useToast();

const username = ref("");
const email = ref("");
const password = ref("");
// An invite link (`/join/<code>`) sends its code along; it also waits in this tab.
const invite = ref(
  typeof route.query.code === "string" ? parseInviteCode(route.query.code) : pendingJoinCode() ?? "",
);
const inviteOnly = computed(() => service.signup !== "open");
const showInvite = ref(inviteOnly.value || Boolean(invite.value));
const show = ref(false);
const error = ref("");
const pending = ref(false);

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.register({
      username: username.value,
      email: email.value.trim(),
      password: password.value,
      inviteCode: parseInviteCode(invite.value),
    });
    clearJoinCode();
    toast.show(t("auth.checkInbox", { email: email.value.trim() }));
    // A club invite made the new account a member: open that club.
    const joined = session.user?.clubs[0];
    if (joined) {
      clubs.remember(joined.slug);
      await router.replace(`/c/${joined.slug}`);
    } else {
      await router.replace("/");
    }
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
    <p class="lede">{{ inviteOnly ? t("auth.signupInviteLede") : t("auth.signupLede") }}</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label v-if="showInvite" class="field">
        <span>{{ inviteOnly ? t("auth.inviteCode") : t("auth.inviteCodeOptional") }}</span>
        <input
          v-model="invite"
          name="invite"
          autocomplete="one-time-code"
          autocapitalize="characters"
          spellcheck="false"
          :required="inviteOnly"
        />
      </label>
      <button v-else class="text-btn invite-toggle" type="button" @click="showInvite = true">
        {{ t("auth.haveInviteCode") }}
      </button>
      <label class="field">
        <span>{{ t("auth.username") }}</span>
        <input
          v-model="username"
          name="username"
          autocomplete="username"
          spellcheck="false"
          required
        />
        <span class="field-hint">{{ t("auth.usernameHint") }}</span>
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
        <span class="field-hint">{{ t("auth.emailHint") }}</span>
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

.invite-toggle {
  margin: 0 0 var(--space-3) calc(-1 * var(--space-2));
}

.auth-alt {
  margin-top: var(--space-5);
}

.auth-locale {
  margin-top: var(--space-5);
}
</style>
