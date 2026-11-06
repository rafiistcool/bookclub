<script setup lang="ts">
import { ArrowDownTrayIcon, DevicePhoneMobileIcon, EnvelopeIcon, KeyIcon, TrashIcon } from "@heroicons/vue/24/outline";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import DeleteAccount from "./DeleteAccount.vue";
import EmailSheet from "./settings/EmailSheet.vue";
import PasswordSheet from "./settings/PasswordSheet.vue";
import SettingsRow from "./settings/SettingsRow.vue";
import { useDeepLinkedSheet } from "./settings/deepLinkedSheet";

/**
 * The account page (`/settings/account`): the email address and its state,
 * the password, the signed-in devices, the data export, and deleting the
 * account at the bottom. Changes happen in sheets; `?sheet=email|password|delete`
 * opens one.
 */
const emit = defineEmits<{ deleted: [] }>();

const { t } = useI18n();
const session = useSession();
const toast = useToast();
const { sheet, open, close } = useDeepLinkedSheet(["email", "password", "delete"] as const);

const user = computed(() => session.user);
/** The address needs the reader: none yet, not confirmed, or a new one waiting. */
const needsAttention = computed(
  () => !user.value?.email || !user.value.email_verified || Boolean(user.value.pending_email),
);
const canResend = computed(() => Boolean(user.value?.pending_email || (user.value?.email && !user.value.email_verified)));

async function resend() {
  try {
    await api.resendVerification();
    toast.show(t("account.resent"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("account.resendFailed"));
  }
}
</script>

<template>
  <ul class="settings-group">
    <li class="email-item">
      <SettingsRow
        class="email-row"
        :icon="EnvelopeIcon"
        :label="t('account.emailHeading')"
        :detail="user?.email || t('settings.noEmail')"
        :alert="needsAttention ? (user?.email ? t('account.unverified') : t('settings.noEmail')) : undefined"
        @click="open('email')"
      />
      <div v-if="needsAttention" class="email-status fine">
        <p v-if="!user?.email" class="muted">{{ t("account.noEmail") }}</p>
        <p v-else-if="user.pending_email" class="pending-line">{{ t("account.pending", { email: user.pending_email }) }}</p>
        <p v-if="canResend" class="muted">
          {{ t("account.checkInbox") }}
          <button class="text-btn" type="button" @click="resend">{{ t("account.resend") }}</button>
        </p>
      </div>
    </li>
    <li>
      <SettingsRow class="password-row" :icon="KeyIcon" :label="t('account.savePassword')" @click="open('password')" />
    </li>
    <li>
      <SettingsRow :icon="DevicePhoneMobileIcon" :label="t('account.sessionsHeading')" to="/settings/devices" />
    </li>
    <li>
      <SettingsRow
        class="export-row"
        :icon="ArrowDownTrayIcon"
        :label="t('settings.exportData')"
        href="/api/auth/me/export"
        download
        :trailing="null"
      />
    </li>
  </ul>

  <ul class="settings-group">
    <li>
      <SettingsRow class="delete-row" :icon="TrashIcon" :label="t('account.deleteHeading')" danger @click="open('delete')" />
    </li>
  </ul>

  <EmailSheet v-if="sheet === 'email'" @close="close" />
  <PasswordSheet v-if="sheet === 'password'" @close="close" />
  <DeleteAccount v-if="sheet === 'delete'" @close="close" @deleted="emit('deleted')" />
</template>

<style scoped>
.settings-group {
  margin: 0;
  padding: 0;
  list-style: none;
}

.settings-group + .settings-group {
  margin-top: var(--space-6);
}

.settings-group li + li {
  border-top: 1px solid var(--border);
}

.email-status {
  display: grid;
  gap: var(--space-1);
  /* Under the label, past the row's icon. */
  padding: 0 var(--space-1) var(--space-3) calc(var(--space-1) + 22px + var(--space-3));
  overflow-wrap: anywhere;
}

.email-status .text-btn {
  font-size: inherit;
}
</style>
