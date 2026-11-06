<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import LegalLinks from "../components/LegalLinks.vue";
import { clearJoinCode, parseInviteCode, pendingJoinCode } from "../invites";
import { useClubs } from "../stores/clubs";
import { useService } from "../stores/service";
import { isTermsChanged, useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { InvitePreview } from "../types";

/**
 * Sign-up. In `open` mode (`ServiceConfig.signup`) anyone may create an
 * account and an invite code is optional (a club invite also joins the
 * club); in `invite` mode the code is required. The browser's time zone and
 * the UI language go along.
 *
 * A code from an invite link stays out of the way: a chip names the club it
 * joins (with "Change code" for another one), so the form asks for three
 * things. The username rule shows only when what was typed breaks it.
 *
 * Above the button, one sentence links the terms and the privacy policy
 * (Q13 option A: no checkbox). Creating the account accepts the terms
 * version this page loaded (`terms_version`), which the server stores with
 * the time. With mail off (`ServiceConfig.mail`) nothing promises a mail.
 */
const { t } = useI18n();
const service = useService();
const session = useSession();
const router = useRouter();
const route = useRoute();
const clubs = useClubs();
const toast = useToast();

/** The rule the server checks, on the name as it stores it (stripped, lowercased). */
const USERNAME = /^[a-z0-9_]{2,32}$/;

const username = ref("");
const usernameLeft = ref(false);
const email = ref("");
const password = ref("");
// An invite link (`/join/<code>`) sends its code along; it also waits in this tab.
const linkCode = typeof route.query.code === "string" ? parseInviteCode(route.query.code) : pendingJoinCode() ?? "";
const invite = ref(linkCode);
const inviteOnly = computed(() => service.signup !== "open");
/** The chip stands in for the code field while the code is the link's. */
const chip = ref(Boolean(linkCode));
const showInvite = ref(!chip.value && (inviteOnly.value || Boolean(invite.value)));
const inviteInput = ref<HTMLInputElement | null>(null);
const preview = ref<InvitePreview | null>(null);
const show = ref(false);
const error = ref("");
const pending = ref(false);

const club = computed(() => (chip.value ? (preview.value?.club ?? null) : null));
const pick = computed(() => (club.value ? (preview.value?.pick ?? null) : null));

/** What was typed breaks the rule: a wrong character or too long at once, too short once the field is left. */
const usernameWrong = computed(() => {
  const name = username.value.trim().toLowerCase();
  if (!name) return false;
  if (USERNAME.test(name)) return false;
  return /[^a-z0-9_]/.test(name) || name.length > 32 || usernameLeft.value;
});

onMounted(async () => {
  if (!linkCode) return;
  try {
    preview.value = (await api.previewInvite(linkCode)) ?? null;
  } catch {
    // An invalid code: the field comes back, and the server says why at sign-up.
    changeCode();
  }
});

async function changeCode() {
  chip.value = false;
  showInvite.value = true;
  await nextTick();
  inviteInput.value?.focus();
}

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.register({
      username: username.value,
      email: email.value.trim(),
      password: password.value,
      inviteCode: parseInviteCode(invite.value),
      termsVersion: service.terms_version,
    });
    clearJoinCode();
    // A club invite made the new account a member: open that club, which greets them.
    const joined = session.user?.clubs[0];
    if (service.mail) toast.show(t("auth.checkInbox", { email: email.value.trim() }));
    else if (!joined) toast.show(t("auth.welcomeNoMail", { name: session.user?.username ?? username.value }));
    if (joined) {
      clubs.remember(joined.slug);
      await router.replace({ path: `/c/${joined.slug}`, state: { joined: true } });
    } else {
      await router.replace("/");
    }
  } catch (err) {
    // New terms came online while the form was open: the next try accepts them.
    if (isTermsChanged(err)) await service.reloadLegal();
    error.value = err instanceof ApiError ? err.message : t("auth.couldNotCreate");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ service.name }}</h1>
    <p v-if="inviteOnly && !chip" class="lede">{{ t("auth.signupInviteLede") }}</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <div v-if="chip" class="invite-chip">
        <span v-if="pick" class="invite-chip-cover">
          <BookCover :book="pick" size="xs" />
        </span>
        <span class="invite-chip-name">{{ club?.name ?? t("auth.inviteAttached") }}</span>
        <button class="text-btn invite-change" type="button" @click="changeCode">{{ t("auth.inviteChange") }}</button>
      </div>
      <label v-else-if="showInvite" class="field">
        <span>{{ inviteOnly ? t("auth.inviteCode") : t("auth.inviteCodeOptional") }}</span>
        <input
          ref="inviteInput"
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
          autocapitalize="none"
          spellcheck="false"
          required
          :aria-invalid="usernameWrong ? 'true' : undefined"
          :aria-describedby="usernameWrong ? 'username-rule' : undefined"
          @blur="usernameLeft = true"
        />
        <span v-if="usernameWrong" id="username-rule" class="field-hint username-rule">{{ t("auth.usernameHint") }}</span>
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
        <span class="field-hint">{{ service.mail ? t("auth.emailHint") : t("auth.emailHintNoMail") }}</span>
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
      <!-- The links open in a new tab, so the form keeps what was typed. -->
      <p v-if="service.hasLegal('terms') || service.hasLegal('privacy')" class="fine muted signup-terms">
        <i18n-t v-if="service.hasLegal('terms')" keypath="terms.signup" tag="span" scope="global">
          <template #terms>
            <RouterLink to="/terms" target="_blank">{{ t("legal.inline.terms") }}</RouterLink>
          </template>
        </i18n-t>
        {{ " " }}
        <i18n-t v-if="service.hasLegal('privacy')" keypath="legal.privacyNote" tag="span" scope="global">
          <template #privacy>
            <RouterLink to="/privacy" target="_blank">{{ t("legal.inline.privacy") }}</RouterLink>
          </template>
        </i18n-t>
      </p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="pending">
        {{ pending ? t("auth.creating") : club ? t("join.createAndJoin") : t("auth.createAccount") }}
      </button>
    </form>
    <p class="muted fine auth-alt">
      {{ t("auth.alreadyHave") }}
      <RouterLink to="/login">{{ t("auth.signIn") }}</RouterLink>
    </p>
    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
    <LegalLinks class="auth-legal" />
  </main>
</template>

<style scoped>
.lede {
  margin-top: var(--space-2);
}

form {
  margin-top: var(--space-6);
}

/* The invite from the link: the club it joins, on one quiet line. */
.invite-chip {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 56px;
  margin-bottom: var(--space-5);
  padding: var(--space-2) 0;
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

.invite-chip-cover {
  flex: none;
  width: 28px;
}

.invite-chip-name {
  flex: 1;
  min-width: 0;
  font-family: var(--serif);
  font-weight: 600;
  overflow-wrap: anywhere;
}

.invite-change {
  flex: none;
  margin-right: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
}

.username-rule {
  color: var(--danger);
}

.signup-terms {
  margin-bottom: var(--space-4);
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

.auth-legal {
  margin-top: var(--space-4);
}
</style>
