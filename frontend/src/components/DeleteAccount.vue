<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useSession } from "../stores/session";
import BottomSheet from "./BottomSheet.vue";

/**
 * Deleting the account, the sheet behind the account page's red row. A club
 * the reader owns with other members in it has to be handed over or deleted
 * first (409 `Transfer or delete your clubs first`); a club of one goes with
 * the account.
 */
const emit = defineEmits<{ close: []; deleted: [] }>();

const { t } = useI18n();
const session = useSession();

const password = ref("");
const pending = ref(false);
const error = ref("");

const ownedWithOthers = computed(() =>
  (session.user?.clubs ?? []).filter((club) => club.role === "owner" && club.member_count > 1),
);

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await api.deleteAccount(password.value);
    session.reset({ leavingDevice: true });
    emit("deleted");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("account.deleteFailed");
  } finally {
    pending.value = false;
  }
}

function close() {
  if (!pending.value) emit("close");
}
</script>

<template>
  <BottomSheet class="delete-account" :title="t('account.deleteHeading')" @close="close">
    <p class="fine muted delete-blurb">{{ t("account.deleteBlurb") }}</p>
    <div v-if="ownedWithOthers.length" class="owned-clubs">
      <p class="fine">{{ t("account.ownedClubs") }}</p>
      <ul>
        <li v-for="club in ownedWithOthers" :key="club.slug">
          <RouterLink :to="`/c/${club.slug}/members`" @click="close">{{ club.name }}</RouterLink>
        </li>
      </ul>
    </div>
    <form class="delete-form" @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("account.deletePassword") }}</span>
        <input v-model="password" type="password" autocomplete="current-password" required />
      </label>
      <button class="btn btn-danger btn-block" type="submit" :disabled="pending || !password">
        {{ t("account.deleteFinal") }}
      </button>
    </form>
  </BottomSheet>
</template>

<style scoped>
.delete-blurb {
  margin-bottom: var(--space-4);
}

.owned-clubs {
  padding: var(--space-3) 0;
  margin-bottom: var(--space-4);
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

.owned-clubs ul {
  margin: var(--space-1) 0 0;
  padding-left: var(--space-5);
}

.owned-clubs li a {
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  font-weight: 600;
}

.delete-form {
  display: grid;
  gap: var(--space-3);
}
</style>
