<script setup lang="ts">
import { PlusIcon } from "@heroicons/vue/24/outline";
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
      <p v-if="!clubs.hasClubs" class="lede">{{ t("clubs.soloLede") }}</p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="clubs.hasClubs && !loaded" class="skeleton clubs-skeleton" aria-hidden="true" />
    <ul v-else-if="clubs.list.length" class="club-grid">
      <li v-for="club in clubs.list" :key="club.slug">
        <ClubCard :club="club" />
      </li>
    </ul>

    <!-- Creating is one more row under the clubs, like the club sheet's "Create a club". -->
    <div class="create-way">
      <RouterLink class="create-row" to="/clubs/new">
        <PlusIcon class="create-icon" aria-hidden="true" />
        <span class="create-text">
          <span class="create-label">{{ t("clubs.create") }}</span>
          <span class="fine subtle">{{ t("clubs.createBlurb") }}</span>
        </span>
      </RouterLink>
    </div>

    <section class="section join-way" aria-labelledby="join-heading">
      <div class="section-head">
        <h2 id="join-heading">{{ t("clubs.joinHeading") }}</h2>
      </div>
      <form class="stack join-form" @submit.prevent="join">
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
        <button class="btn btn-ghost" type="submit" :disabled="!code.trim()">{{ t("clubs.join") }}</button>
      </form>
    </section>
  </section>
</template>

<style scoped>
.clubs-page {
  max-width: 720px;
}

.clubs-skeleton {
  height: 120px;
}

/* The clubs and the create row are light rows split by hairlines. */
.club-grid {
  list-style: none;
  margin: 0;
  padding: 0;
}

.club-grid > li + li,
.club-grid + .create-way {
  border-top: 1px solid var(--border);
}

.create-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 56px;
  padding: var(--space-3) 0;
  color: inherit;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.create-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

/* Centred in a box as wide as a club row's cover (52px), so the label lines
   up with the club names above it. */
.create-icon {
  flex: none;
  box-sizing: content-box;
  width: 22px;
  height: 22px;
  padding-inline: 15px;
  color: var(--accent);
}

.create-text {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.create-label {
  color: var(--accent);
  font-weight: 600;
}

@media (hover: hover) {
  .create-row:hover .create-label {
    text-decoration: underline;
  }
}

.join-form {
  max-width: 480px;
}

.join-form > .btn {
  justify-self: start;
}
</style>
