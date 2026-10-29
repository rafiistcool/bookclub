<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { isClubAdmin, useClubs } from "../stores/clubs";
import BottomSheet from "./BottomSheet.vue";

/**
 * Invites a friend into one of the caller's clubs: a personal, single-use
 * invite (`kind: "friend"`) that only that friend can redeem, from their
 * Friends page. Offered for the clubs the caller owns or administers (the
 * session's clubs). A repeat answers with the open invite, so a second tap
 * sends nothing new.
 */
const props = defineProps<{ username: string }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const clubs = useClubs();

const adminClubs = computed(() => clubs.items.filter((club) => isClubAdmin(club.role)));
/** Per club slug: the request running, the invitation sent, or why it failed. */
const states = ref<Record<string, { busy?: boolean; sent?: boolean; error?: string }>>({});
const busy = computed(() => Object.values(states.value).some((state) => state.busy));

async function invite(slug: string) {
  if (busy.value) return;
  states.value = { ...states.value, [slug]: { busy: true } };
  try {
    await api.createInvite(slug, { kind: "friend", username: props.username });
    states.value = { ...states.value, [slug]: { sent: true } };
  } catch (err) {
    // 404 `No friend with that name`, 409 `That friend is already in this club`: translated by the client.
    const error = err instanceof ApiError ? err.message : t("friends.inviteSheet.failed");
    states.value = { ...states.value, [slug]: { error } };
  }
}
</script>

<template>
  <BottomSheet :title="t('friends.inviteSheet.title', { name: username })" @close="emit('close')">
    <div class="stack">
      <p v-if="adminClubs.length" class="fine muted invite-blurb">{{ t("friends.inviteSheet.blurb", { name: username }) }}</p>
      <p v-else class="fine muted invite-blurb">{{ t("friends.inviteSheet.none") }}</p>
      <ul v-if="adminClubs.length" class="invite-clubs">
        <li v-for="club in adminClubs" :key="club.slug" class="invite-club">
          <span class="invite-club-row">
            <strong class="invite-club-name">{{ club.name }}</strong>
            <span v-if="states[club.slug]?.sent" class="fine invite-sent" role="status">
              {{ t("friends.inviteSheet.sent") }}
            </span>
            <button
              v-else
              class="btn btn-primary btn-sm"
              type="button"
              :disabled="busy"
              @click="invite(club.slug)"
            >
              {{ t("friends.inviteSheet.invite") }}
            </button>
          </span>
          <p v-if="states[club.slug]?.error" class="error" role="alert">{{ states[club.slug]?.error }}</p>
        </li>
      </ul>
      <button class="btn btn-ghost btn-block" type="button" @click="emit('close')">
        {{ t("friends.inviteSheet.done") }}
      </button>
    </div>
  </BottomSheet>
</template>

<style scoped>
.invite-blurb {
  margin: 0;
}

.invite-clubs {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.invite-club {
  display: grid;
  gap: var(--space-1);
  padding: var(--space-2) var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.invite-club-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-height: var(--tap);
}

.invite-club-name {
  min-width: 0;
  overflow-wrap: anywhere;
}

.invite-sent {
  color: var(--accent);
  font-weight: 600;
}
</style>
