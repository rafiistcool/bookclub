<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BottomSheet from "../BottomSheet.vue";

/**
 * A new password. The server signs the other devices out and voids a pending
 * address of a verified account, so the account is read again afterwards:
 * otherwise "Send it again" would say "Link sent" for a link that never goes out.
 */
const emit = defineEmits<{ close: []; saved: [] }>();

const { t } = useI18n();
const session = useSession();
const toast = useToast();

const current = ref("");
const next = ref("");
const busy = ref(false);
const error = ref("");

async function save() {
  error.value = "";
  busy.value = true;
  try {
    await api.changePassword(current.value, next.value);
    toast.show(t("account.passwordSaved"));
    emit("saved");
    emit("close");
    try {
      session.user = await api.me();
    } catch {
      /* The next load catches up. */
    }
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("account.passwordFailed");
  } finally {
    busy.value = false;
  }
}

function close() {
  if (!busy.value) emit("close");
}
</script>

<template>
  <BottomSheet class="password-sheet" :title="t('account.savePassword')" @close="close">
    <form class="sheet-form" @submit.prevent="save">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("settings.currentPassword") }}</span>
        <input v-model="current" type="password" autocomplete="current-password" required />
      </label>
      <label class="field">
        <span>{{ t("account.newPassword") }}</span>
        <input v-model="next" type="password" autocomplete="new-password" minlength="8" required />
        <span class="field-hint">{{ t("account.passwordHint") }}</span>
      </label>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy">{{ t("account.savePassword") }}</button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.sheet-form {
  display: grid;
  gap: var(--space-3);
}
</style>
