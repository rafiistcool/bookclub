<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useSession } from "../stores/session";

/**
 * Deleting the account. A club the reader owns with other members in it has
 * to be handed over or deleted first (409 `Transfer or delete your clubs
 * first`); a club of one goes with the account.
 */
const emit = defineEmits<{ deleted: [] }>();

const { t } = useI18n();
const session = useSession();

const open = ref(false);
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
    session.reset();
    emit("deleted");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("account.deleteFailed");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <section class="section delete-account" aria-labelledby="delete-account">
    <div class="section-head">
      <h2 id="delete-account">{{ t("account.deleteHeading") }}</h2>
    </div>
    <p class="fine muted delete-blurb">{{ t("account.deleteBlurb") }}</p>
    <div v-if="ownedWithOthers.length" class="panel owned-clubs">
      <p class="fine">{{ t("account.ownedClubs") }}</p>
      <ul>
        <li v-for="club in ownedWithOthers" :key="club.slug">
          <RouterLink :to="`/c/${club.slug}/members`">{{ club.name }}</RouterLink>
        </li>
      </ul>
    </div>
    <button v-if="!open" class="btn btn-danger btn-sm" type="button" @click="open = true">
      {{ t("account.delete") }}
    </button>
    <form v-else class="panel delete-form" @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("account.deletePassword") }}</span>
        <input v-model="password" type="password" autocomplete="current-password" required />
      </label>
      <div class="btn-row">
        <button class="btn btn-danger btn-sm" type="submit" :disabled="pending || !password">
          {{ t("account.deleteFinal") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="open = false; password = ''">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>
  </section>
</template>

<style scoped>
.delete-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.owned-clubs {
  margin-bottom: var(--space-3);
}

.owned-clubs ul {
  margin: var(--space-1) 0 0;
  padding-left: var(--space-5);
}

.delete-form {
  max-width: 480px;
}
</style>
