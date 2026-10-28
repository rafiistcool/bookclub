<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { describeUserAgent } from "../devices";
import { formatClubDate } from "../i18n/dates";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { NotificationsOut, SessionOut } from "../api/generated";

/** Settings of the account itself: the email address, the password, signed-in devices, notifications. */
const emit = defineEmits<{ loggedOut: [] }>();

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const user = computed(() => session.user);

function message(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

// ---------------------------------------------------------------- email
const changingEmail = ref(false);
const newEmail = ref("");
const emailPassword = ref("");
const emailBusy = ref(false);
const emailError = ref("");

async function saveEmail() {
  emailError.value = "";
  emailBusy.value = true;
  try {
    const address = newEmail.value.trim();
    await api.changeEmail(address, emailPassword.value);
    session.user = await api.me();
    changingEmail.value = false;
    newEmail.value = "";
    emailPassword.value = "";
    // The server answers the same whether or not the address is taken.
    toast.show(t("account.emailSent", { email: address }));
  } catch (err) {
    emailError.value = message(err, t("account.emailFailed"));
  } finally {
    emailBusy.value = false;
  }
}

async function resend() {
  try {
    await api.resendVerification();
    toast.show(t("account.resent"));
  } catch (err) {
    toast.show(message(err, t("account.resendFailed")));
  }
}

// ------------------------------------------------------------- password
const currentPassword = ref("");
const nextPassword = ref("");
const passwordBusy = ref(false);
const passwordError = ref("");

async function savePassword() {
  passwordError.value = "";
  passwordBusy.value = true;
  try {
    await api.changePassword(currentPassword.value, nextPassword.value);
    currentPassword.value = "";
    nextPassword.value = "";
    toast.show(t("account.passwordSaved"));
    void loadSessions();
  } catch (err) {
    passwordError.value = message(err, t("account.passwordFailed"));
  } finally {
    passwordBusy.value = false;
  }
}

// ------------------------------------------------------------- sessions
const sessions = ref<SessionOut[]>([]);
const sessionsLoaded = ref(false);
const sessionsError = ref("");

async function loadSessions() {
  try {
    sessions.value = (await api.sessions()).items;
    sessionsError.value = "";
  } catch (err) {
    sessionsError.value = message(err, t("account.sessionsFailed"));
  } finally {
    sessionsLoaded.value = true;
  }
}

async function endSession(row: SessionOut) {
  try {
    await api.endSession(row.id);
    if (row.current) {
      session.reset();
      emit("loggedOut");
      return;
    }
    sessions.value = sessions.value.filter((item) => item.id !== row.id);
    toast.show(t("account.sessionEnded"));
  } catch (err) {
    toast.show(message(err, t("account.sessionsFailed")));
  }
}

/** "Firefox on Linux" for a browser; an app's own label; the raw User-Agent when it says too little. */
function deviceName(row: { kind: string; label: string }): string {
  if (row.kind === "bearer") return row.label || t("account.app");
  if (!row.label) return t("account.browser");
  const { browser, os } = describeUserAgent(row.label);
  if (browser && os) return t("account.deviceOn", { browser, os });
  return browser ?? row.label;
}

function lastUsed(row: SessionOut): string {
  return formatClubDate(row.last_used_at, user.value?.timezone || "UTC", locale.locale);
}

// -------------------------------------------------------- notifications
const notifications = ref<NotificationsOut | null>(null);

async function loadNotifications() {
  try {
    notifications.value = await api.notifications();
  } catch {
    notifications.value = null;
  }
}

async function toggle(key: keyof NotificationsOut, value: boolean) {
  const before = notifications.value;
  if (!before) return;
  notifications.value = { ...before, [key]: value };
  try {
    notifications.value = await api.saveNotifications({ [key]: value });
    toast.show(t("account.notificationsSaved"));
  } catch (err) {
    notifications.value = before;
    toast.show(message(err, t("account.notificationsFailed")));
  }
}

const NOTIFICATION_KEYS = ["notify_meeting", "notify_pick", "notify_note"] as const;

onMounted(() => {
  void loadSessions();
  void loadNotifications();
});
</script>

<template>
  <section class="section" aria-labelledby="account-email">
    <div class="section-head">
      <h2 id="account-email">{{ t("account.emailHeading") }}</h2>
    </div>
    <div class="panel account-panel">
      <p v-if="user?.email" class="email-line">
        <strong>{{ user.email }}</strong>
        <span class="badge" :class="user.email_verified ? 'finished' : 'want_to_read'">
          {{ user.email_verified ? t("account.verified") : t("account.unverified") }}
        </span>
      </p>
      <p v-else class="fine muted">{{ t("account.noEmail") }}</p>
      <p v-if="user?.pending_email" class="fine pending-line">
        {{ t("account.pending", { email: user.pending_email }) }}
      </p>
      <p v-if="user?.pending_email || (user?.email && !user.email_verified)" class="fine muted">
        {{ t("account.checkInbox") }}
        <button class="text-btn" type="button" @click="resend">{{ t("account.resend") }}</button>
      </p>

      <form v-if="changingEmail" class="account-form" @submit.prevent="saveEmail">
        <p v-if="emailError" class="error">{{ emailError }}</p>
        <label class="field">
          <span>{{ t("account.newEmail") }}</span>
          <input v-model="newEmail" type="email" autocomplete="email" spellcheck="false" required />
        </label>
        <label class="field">
          <span>{{ t("settings.currentPassword") }}</span>
          <input v-model="emailPassword" type="password" autocomplete="current-password" required />
        </label>
        <p class="field-hint">{{ t("account.emailHint") }}</p>
        <div class="btn-row">
          <button class="btn btn-primary btn-sm" type="submit" :disabled="emailBusy">{{ t("account.saveEmail") }}</button>
          <button class="btn btn-ghost btn-sm" type="button" @click="changingEmail = false">{{ t("common.cancel") }}</button>
        </div>
      </form>
      <button v-else class="btn btn-ghost btn-sm" type="button" @click="changingEmail = true">
        {{ user?.email ? t("account.changeEmail") : t("account.addEmail") }}
      </button>
    </div>
  </section>

  <section class="section" aria-labelledby="account-password">
    <div class="section-head">
      <h2 id="account-password">{{ t("account.passwordHeading") }}</h2>
    </div>
    <form class="panel account-form" @submit.prevent="savePassword">
      <p v-if="passwordError" class="error">{{ passwordError }}</p>
      <label class="field">
        <span>{{ t("settings.currentPassword") }}</span>
        <input v-model="currentPassword" type="password" autocomplete="current-password" required />
      </label>
      <label class="field">
        <span>{{ t("account.newPassword") }}</span>
        <input v-model="nextPassword" type="password" autocomplete="new-password" minlength="8" required />
        <span class="field-hint">{{ t("account.passwordHint") }}</span>
      </label>
      <button class="btn btn-primary btn-sm" type="submit" :disabled="passwordBusy">{{ t("account.savePassword") }}</button>
    </form>
  </section>

  <section class="section" aria-labelledby="account-sessions">
    <div class="section-head">
      <h2 id="account-sessions">{{ t("account.sessionsHeading") }}</h2>
    </div>
    <p v-if="sessionsError" class="error">{{ sessionsError }}</p>
    <div v-else-if="!sessionsLoaded" class="skeleton sessions-skeleton" aria-hidden="true" />
    <ul v-else class="session-list">
      <li v-for="row in sessions" :key="row.id" class="session-row">
        <span class="session-meta">
          <strong class="session-label" :title="row.label || undefined">{{ deviceName(row) }}</strong>
          <span class="finer subtle">
            <template v-if="row.current">{{ t("account.thisDevice") }} · </template>
            {{ t("account.lastUsed", { when: lastUsed(row) }) }}
          </span>
        </span>
        <button class="text-btn" type="button" @click="endSession(row)">
          {{ row.current ? t("account.signOutHere") : t("account.signOut") }}
        </button>
      </li>
    </ul>
  </section>

  <section v-if="notifications" class="section" aria-labelledby="account-notifications">
    <div class="section-head">
      <h2 id="account-notifications">{{ t("account.notificationsHeading") }}</h2>
    </div>
    <p class="fine muted notifications-blurb">{{ t("account.notificationsBlurb") }}</p>
    <div class="panel notification-list">
      <label v-for="key in NOTIFICATION_KEYS" :key="key" class="notification-row">
        <input
          type="checkbox"
          :checked="notifications[key]"
          @change="toggle(key, ($event.target as HTMLInputElement).checked)"
        />
        {{ t(`account.${key}`) }}
      </label>
    </div>
  </section>
</template>

<style scoped>
.account-panel {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.email-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  overflow-wrap: anywhere;
}

.pending-line {
  overflow-wrap: anywhere;
}

.account-form {
  max-width: 480px;
  width: 100%;
}

.sessions-skeleton {
  height: 90px;
}

.session-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.session-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.session-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.session-label {
  font-size: var(--text-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.notifications-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.notification-list {
  display: grid;
  gap: var(--space-1);
}

.notification-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--tap);
}
</style>
