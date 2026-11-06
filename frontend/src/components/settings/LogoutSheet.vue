<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useSession } from "../../stores/session";
import BottomSheet from "../BottomSheet.vue";

/** "Log out?": a red row is easy to hit by mistake, so it asks once. */
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const router = useRouter();
const session = useSession();
const busy = ref(false);

async function logout() {
  if (busy.value) return;
  busy.value = true;
  try {
    await session.logout();
  } catch {
    // The session is forgotten here either way (session.logout resets it).
  }
  busy.value = false;
  await router.push("/login");
}

function close() {
  if (!busy.value) emit("close");
}
</script>

<template>
  <BottomSheet class="logout-sheet" :title="t('settings.logOutTitle')" @close="close">
    <p class="muted logout-text">{{ t("settings.logOutText") }}</p>
    <div class="logout-actions">
      <button class="btn btn-danger btn-block" type="button" :disabled="busy" @click="logout">
        {{ t("settings.logOut") }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="close">
        {{ t("common.cancel") }}
      </button>
    </div>
  </BottomSheet>
</template>

<style scoped>
.logout-text {
  margin-bottom: var(--space-4);
}

.logout-actions {
  display: grid;
  gap: var(--space-2);
}
</style>
