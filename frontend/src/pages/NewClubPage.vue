<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import { useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { timeZoneOptions } from "../timezone";

const { t } = useI18n();
const session = useSession();
const clubs = useClubs();
const toast = useToast();
const router = useRouter();

const name = ref("");
const timezone = ref(session.user?.timezone || "UTC");
const zones = computed(() => timeZoneOptions(timezone.value));
const pending = ref(false);
const error = ref("");
/** The server asks for a verified address before a club is created. */
const unverified = computed(() => !session.user?.email_verified);

async function submit() {
  const value = name.value.trim();
  if (!value || pending.value) return;
  pending.value = true;
  error.value = "";
  try {
    const club = await api.createClub({ name: value, timezone: timezone.value });
    clubs.upsert(club);
    clubs.remember(club.slug);
    toast.show(t("newClub.created", { club: club.name }));
    // An empty club's first job is to invite people.
    await router.replace(`/c/${club.slug}/members`);
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("newClub.failed");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <section class="new-club">
    <div class="page-head">
      <h1>{{ t("newClub.title") }}</h1>
      <p class="lede">{{ t("newClub.lede") }}</p>
    </div>

    <p v-if="unverified" class="panel fine verify-note">
      {{ session.user?.email ? t("newClub.verifyFirst", { email: session.user.email }) : t("newClub.addEmailFirst") }}
      <RouterLink to="/settings">{{ t("newClub.toSettings") }}</RouterLink>
    </p>

    <form class="new-club-form" @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>{{ t("newClub.name") }}</span>
        <input v-model="name" name="club-name" maxlength="80" required :placeholder="t('newClub.namePlaceholder')" />
      </label>
      <label class="field">
        <span>{{ t("clubSettings.timezone") }}</span>
        <select v-model="timezone">
          <option v-for="zone in zones" :key="zone" :value="zone">{{ zone }}</option>
        </select>
        <span class="field-hint">{{ t("clubSettings.timezoneHint") }}</span>
      </label>
      <div class="btn-row">
        <button class="btn btn-primary" type="submit" :disabled="pending || !name.trim()">
          {{ pending ? t("newClub.creating") : t("newClub.create") }}
        </button>
        <RouterLink class="btn btn-ghost" to="/clubs">{{ t("common.cancel") }}</RouterLink>
      </div>
    </form>
  </section>
</template>

<style scoped>
.new-club {
  max-width: 520px;
}

.verify-note {
  margin-bottom: var(--space-4);
}
</style>
