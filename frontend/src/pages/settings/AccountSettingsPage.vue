<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AccountSettings from "../../components/AccountSettings.vue";
import BackLink from "../../components/BackLink.vue";
import { useToast } from "../../stores/toast";

/** `/settings/account`: the address, the password, the devices, the export, deleting the account. */
const { t } = useI18n();
const router = useRouter();
const toast = useToast();

function deleted() {
  toast.show(t("account.deleted"));
  void router.replace("/login");
}
</script>

<template>
  <section class="settings-sub">
    <BackLink :label="t('settings.back')" fallback="/settings" />
    <div class="page-head">
      <h1>{{ t("settings.account") }}</h1>
    </div>
    <AccountSettings @deleted="deleted" />
  </section>
</template>

<style scoped>
.settings-sub {
  max-width: 560px;
}
</style>
