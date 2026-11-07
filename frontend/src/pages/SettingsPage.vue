<script setup lang="ts">
import {
  ArrowDownTrayIcon,
  ArrowRightStartOnRectangleIcon,
  GlobeEuropeAfricaIcon,
  LanguageIcon,
  ScaleIcon,
  SwatchIcon,
  UserCircleIcon,
} from "@heroicons/vue/24/outline";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import AdminLink from "../components/admin/AdminLink.vue";
import BackLink from "../components/BackLink.vue";
import BottomSheet from "../components/BottomSheet.vue";
import TimeZoneSetting from "../components/TimeZoneSetting.vue";
import AppearanceSheet from "../components/settings/AppearanceSheet.vue";
import LanguageSheet from "../components/settings/LanguageSheet.vue";
import LegalSheet from "../components/settings/LegalSheet.vue";
import LogoutSheet from "../components/settings/LogoutSheet.vue";
import SettingsRow from "../components/settings/SettingsRow.vue";
import { useDeepLinkedSheet } from "../components/settings/deepLinkedSheet";
import { useLocale } from "../stores/locale";
import { LEGAL_DOCS, useService } from "../stores/service";
import { useSession } from "../stores/session";
import { MODES, THEMES, useTheme } from "../stores/theme";

/**
 * Settings: a short index. A choice that applies at once opens a sheet
 * (appearance, language, time zone, legal, logging out); the account and the
 * import are pages of their own (`settingsRoutes.ts`). Each row shows its
 * current value, so a change shows behind the sheet as it is made. The
 * profile picture and the favourites are edited on the profile.
 */
const SHEETS = ["appearance", "language", "timezone", "legal", "logout"] as const;

const { t } = useI18n();
const theme = useTheme();
const locale = useLocale();
const session = useSession();
const service = useService();
const { sheet, open, close } = useDeepLinkedSheet(SHEETS);

const user = computed(() => session.user);

const appearance = computed(() => {
  const palette = THEMES.find((option) => option.id === theme.theme) ?? THEMES[0];
  const mode = MODES.find((option) => option.id === theme.mode) ?? MODES[2];
  return `${t(palette.labelKey)} · ${t(mode.labelKey)}`;
});

const accountAlert = computed(() => {
  const me = user.value;
  if (!me) return undefined;
  if (!me.email) return t("settings.noEmail");
  // With mail off no address can be confirmed, so none is asked for.
  if (!service.mail) return undefined;
  if (!me.email_verified) return t("account.unverified");
  // The current address may be confirmed: the alert names the one waiting.
  if (me.pending_email) return t("account.pending", { email: me.pending_email });
  return undefined;
});

const hasLegal = computed(() => LEGAL_DOCS.some((doc) => service.hasLegal(doc)) || Boolean(service.contact_email));
</script>

<template>
  <section class="settings-index">
    <BackLink :label="t('settings.backToProfile')" fallback="/me" exact />
    <div class="page-head">
      <h1>{{ t("settings.title") }}</h1>
    </div>

    <ul class="settings-group">
      <li>
        <SettingsRow class="appearance-row" :icon="SwatchIcon" :label="t('settings.appearance')" :value="appearance" @click="open('appearance')" />
      </li>
      <li>
        <SettingsRow
          class="language-row"
          :icon="LanguageIcon"
          :label="t('settings.language')"
          :value="t(`locale.${locale.locale}`)"
          @click="open('language')"
        />
      </li>
      <li>
        <SettingsRow
          class="timezone-row"
          :icon="GlobeEuropeAfricaIcon"
          :label="t('settings.timezone')"
          :value="user?.timezone || 'UTC'"
          @click="open('timezone')"
        />
      </li>
    </ul>

    <ul class="settings-group">
      <li>
        <SettingsRow
          class="account-row"
          :icon="UserCircleIcon"
          :label="t('settings.account')"
          :value="user?.email || undefined"
          :alert="accountAlert"
          to="/settings/account"
        />
      </li>
      <li>
        <SettingsRow
          class="import-row"
          :icon="ArrowDownTrayIcon"
          :label="t('settings.importRow')"
          :value="t('settings.importSource')"
          to="/settings/import"
        />
      </li>
    </ul>

    <ul v-if="user?.is_admin || hasLegal" class="settings-group">
      <li v-if="user?.is_admin">
        <AdminLink />
      </li>
      <li v-if="hasLegal">
        <SettingsRow class="legal-row" :icon="ScaleIcon" :label="t('legal.settingsTitle')" @click="open('legal')" />
      </li>
    </ul>

    <ul class="settings-group">
      <li>
        <SettingsRow
          class="logout-row"
          :icon="ArrowRightStartOnRectangleIcon"
          :label="t('settings.logOut')"
          danger
          @click="open('logout')"
        />
      </li>
    </ul>

    <AppearanceSheet v-if="sheet === 'appearance'" @close="close" />
    <LanguageSheet v-if="sheet === 'language'" @close="close" />
    <BottomSheet v-if="sheet === 'timezone'" class="timezone-sheet" :title="t('settings.timezone')" @close="close">
      <TimeZoneSetting hide-label />
    </BottomSheet>
    <LegalSheet v-if="sheet === 'legal' && hasLegal" @close="close" />
    <LogoutSheet v-if="sheet === 'logout'" @close="close" />
  </section>
</template>

<style scoped>
.settings-index {
  max-width: 560px;
}

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
</style>
