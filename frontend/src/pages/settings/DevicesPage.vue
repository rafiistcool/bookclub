<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import BackLink from "../../components/BackLink.vue";
import SessionList from "../../components/settings/SessionList.vue";

/** `/settings/devices`: where the account is signed in; ending this device's session leaves the app. */
const { t } = useI18n();
const router = useRouter();
</script>

<template>
  <section class="settings-sub">
    <BackLink :label="t('settings.backToAccount')" fallback="/settings/account" />
    <div class="page-head">
      <h1>{{ t("account.sessionsHeading") }}</h1>
    </div>
    <SessionList @logged-out="router.replace('/login')" />
  </section>
</template>

<style scoped>
.settings-sub {
  max-width: 560px;
}
</style>
