<script setup lang="ts">
import { ChevronRightIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import BottomSheet from "./BottomSheet.vue";

/**
 * Deleting the account, the sheet behind the account page's red row. A club
 * the reader owns with other members in it has to be handed over or deleted
 * first (409 `Transfer or delete your clubs first`); a club of one goes with
 * the account. While such a club is listed, the form stays disabled, so the
 * reader does not type the password only to hear that; the list is read
 * fresh when the sheet opens, and the server still has the last word.
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
const blocked = computed(() => ownedWithOthers.value.length > 0);

onMounted(() => {
  // Someone may have joined or left a club since the account was read.
  useClubs()
    .refresh()
    .catch(() => {});
});

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
    <div v-if="blocked" class="owned-clubs">
      <p class="fine">{{ t("account.ownedClubs") }}</p>
      <ul class="owned-list">
        <li v-for="club in ownedWithOthers" :key="club.slug">
          <RouterLink class="owned-row" :to="`/c/${club.slug}/members`" @click="close">
            <span class="owned-name">{{ club.name }}</span>
            <ChevronRightIcon class="owned-chevron" aria-hidden="true" />
          </RouterLink>
        </li>
      </ul>
    </div>
    <form class="delete-form" @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <fieldset class="delete-fields" :disabled="blocked">
        <label class="field">
          <span>{{ t("account.deletePassword") }}</span>
          <input v-model="password" type="password" autocomplete="current-password" required />
        </label>
        <button class="btn btn-danger btn-block" type="submit" :disabled="pending || !password || blocked">
          {{ t("account.deleteFinal") }}
        </button>
      </fieldset>
    </form>
  </BottomSheet>
</template>

<style scoped>
.delete-blurb {
  margin-bottom: var(--space-4);
}

.owned-clubs {
  margin-bottom: var(--space-4);
}

.owned-clubs p {
  margin: 0 0 var(--space-1);
}

/* The clubs as rows like the sheet's others: name and chevron, hairlines between. */
.owned-list {
  margin: 0;
  padding: 0;
  list-style: none;
  border-block: 1px solid var(--border);
}

.owned-list li + li {
  border-top: 1px solid var(--border);
}

.owned-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 52px;
  color: var(--text);
  font-weight: 600;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.owned-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .owned-row:hover .owned-name {
    color: var(--accent);
  }
}

.owned-name {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}

.owned-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}

.delete-form,
.delete-fields {
  display: grid;
  gap: var(--space-3);
}

.delete-fields {
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
}

.delete-fields .field {
  margin-bottom: 0;
}

.delete-fields:disabled .field {
  opacity: 0.55;
}
</style>
