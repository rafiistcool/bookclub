<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { ApiError } from "../api/client";
import ClubCard from "../components/ClubCard.vue";
import { parseInviteCode } from "../invites";
import { useClubs } from "../stores/clubs";

/** The viewer's clubs, and the way into a new one: create it or join with a code. */
const { t } = useI18n();
const clubs = useClubs();
const router = useRouter();

const loaded = ref(false);
const error = ref("");
const code = ref("");

async function load() {
  try {
    await clubs.loadList();
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("clubs.loadFailed");
  } finally {
    loaded.value = true;
  }
}

function join() {
  const value = parseInviteCode(code.value);
  if (value) void router.push(`/join/${encodeURIComponent(value)}`);
}

onMounted(load);
</script>

<template>
  <section class="clubs-page">
    <div class="page-head">
      <h1>{{ t("clubs.title") }}</h1>
      <p class="lede">{{ clubs.hasClubs ? t("clubs.lede") : t("clubs.soloLede") }}</p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="clubs.hasClubs && !loaded" class="skeleton clubs-skeleton" aria-hidden="true" />
    <ul v-else-if="clubs.list.length" class="club-grid">
      <li v-for="club in clubs.list" :key="club.slug">
        <ClubCard :club="club" />
      </li>
    </ul>

    <div class="club-ways">
      <section class="panel">
        <h2>{{ t("clubs.createHeading") }}</h2>
        <p class="fine muted">{{ t("clubs.createBlurb") }}</p>
        <RouterLink class="btn btn-primary btn-sm" to="/clubs/new">{{ t("clubs.create") }}</RouterLink>
      </section>
      <form class="panel" @submit.prevent="join">
        <h2>{{ t("clubs.joinHeading") }}</h2>
        <label class="field">
          <span>{{ t("clubs.joinLabel") }}</span>
          <input
            v-model="code"
            name="invite"
            autocomplete="off"
            autocapitalize="characters"
            spellcheck="false"
            :placeholder="t('clubs.joinPlaceholder')"
          />
        </label>
        <button class="btn btn-ghost btn-sm" type="submit" :disabled="!code.trim()">{{ t("clubs.join") }}</button>
      </form>
    </div>
  </section>
</template>

<style scoped>
.clubs-skeleton {
  height: 120px;
}

.club-grid {
  list-style: none;
  margin: 0 0 var(--space-6);
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.club-ways {
  display: grid;
  gap: var(--space-3);
}

.club-ways .panel {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.club-ways h2 {
  font-size: var(--text-lg);
}

.club-ways .field {
  width: 100%;
  margin-bottom: var(--space-1);
}

@media (min-width: 720px) {
  .club-grid,
  .club-ways {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
