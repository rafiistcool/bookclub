<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import AccountSettings from "../components/AccountSettings.vue";
import AdminLink from "../components/admin/AdminLink.vue";
import Avatar from "../components/Avatar.vue";
import DeleteAccount from "../components/DeleteAccount.vue";
import FavoritesEditor from "../components/FavoritesEditor.vue";
import GoodreadsImport from "../components/GoodreadsImport.vue";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import TimeZoneSetting from "../components/TimeZoneSetting.vue";
import { MODES, THEMES, useTheme } from "../stores/theme";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { Favorite } from "../types";

const { t } = useI18n();
const router = useRouter();
const theme = useTheme();
const session = useSession();
const toast = useToast();
const avatarBusy = ref(false);


const username = computed(() => session.user?.username ?? "");
const favorites = ref<Favorite[]>([]);
const favoritesBusy = ref(false);

async function loadFavorites() {
  try {
    favorites.value = (await api.myFavorites()).items;
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("favorites.saveFailed"));
  }
}

async function replaceFavorites(workIds: number[]) {
  if (favoritesBusy.value) return;
  favoritesBusy.value = true;
  try {
    favorites.value = (await api.replaceFavorites(workIds)).items;
    toast.show(workIds.length ? t("favorites.saved") : t("favorites.cleared"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("favorites.saveFailed"));
  } finally {
    favoritesBusy.value = false;
  }
}

async function onAvatarFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file || avatarBusy.value) return;
  if (file.size > 50 * 1024 * 1024) {
    toast.show(t("errors.pictureTooLarge"));
    return;
  }
  avatarBusy.value = true;
  try {
    session.user = await api.uploadAvatar(file);
    toast.show(t("settings.photoSaved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("settings.photoFailed"));
  } finally {
    avatarBusy.value = false;
  }
}

async function removeAvatar() {
  if (avatarBusy.value) return;
  avatarBusy.value = true;
  try {
    session.user = await api.deleteAvatar();
    toast.show(t("settings.photoRemoved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("settings.photoFailed"));
  } finally {
    avatarBusy.value = false;
  }
}

async function logout() {
  await session.logout();
  await router.push("/login");
}

/** The session already ended on the server (signed out here, or the account is gone). */
function toLogin(message?: string) {
  if (message) toast.show(message);
  void router.replace("/login");
}

onMounted(() => {
  void loadFavorites();
});
</script>

<template>
  <section>
    <div class="page-head">
      <h1>{{ t("settings.title") }}</h1>
      <p class="lede">{{ t("settings.signedInAs", { name: username }) }}</p>
    </div>

    <section aria-labelledby="profile">
      <div class="section-head">
        <h2 id="profile">{{ t("settings.profile") }}</h2>
      </div>
      <p class="fine muted">{{ t("settings.profileBlurb") }}</p>
      <div class="profile-row">
        <Avatar :username="username" :src="session.user?.avatar_url" size="lg" />
        <div class="profile-actions">
          <label class="btn btn-ghost">
            <input
              type="file"
              accept="image/jpeg,image/png,image/webp,image/gif"
              :disabled="avatarBusy"
              @change="onAvatarFile"
            />
            {{ t("settings.choosePhoto") }}
          </label>
          <button
            v-if="session.user?.avatar_url"
            class="btn btn-ghost"
            type="button"
            :disabled="avatarBusy"
            @click="removeAvatar"
          >
            {{ t("settings.removePhoto") }}
          </button>
        </div>
      </div>
    </section>

    <AccountSettings @logged-out="toLogin()" />

    <section class="section" aria-labelledby="favorites">
      <div class="section-head">
        <h2 id="favorites">{{ t("favorites.heading") }}</h2>
      </div>
      <p class="fine muted">{{ t("favorites.settingsBlurb") }}</p>
      <FavoritesEditor :items="favorites" :busy="favoritesBusy" @replace="replaceFavorites" />
    </section>

    <section class="section" aria-labelledby="language">
      <div class="section-head">
        <h2 id="language">{{ t("settings.language") }}</h2>
      </div>
      <p class="fine muted">{{ t("settings.languageBlurb") }}</p>
      <div class="mode-row">
        <LanguageSwitch />
      </div>
    </section>

    <TimeZoneSetting />

    <section aria-labelledby="appearance">
      <div class="section-head">
        <h2 id="appearance">{{ t("settings.appearance") }}</h2>
      </div>

      <div class="mode-row">
        <div class="segmented" role="group" :aria-label="t('settings.colourMode')">
          <button
            v-for="option in MODES"
            :key="option.id"
            type="button"
            :aria-pressed="theme.mode === option.id"
            @click="theme.setMode(option.id)"
          >
            {{ t(option.labelKey) }}
          </button>
        </div>
        <p class="fine subtle">
          <template v-if="theme.mode === 'system'">
            {{ t("settings.followingDevice", { mode: t(`theme.${theme.resolvedMode}`) }) }}
          </template>
          <template v-else>{{ t("settings.alwaysMode", { mode: t(`theme.${theme.mode}`) }) }}</template>
        </p>
      </div>

      <div class="theme-grid">
        <button
          v-for="option in THEMES"
          :key="option.id"
          class="theme-card"
          :class="{ active: theme.theme === option.id }"
          type="button"
          :aria-pressed="theme.theme === option.id"
          @click="theme.setTheme(option.id)"
        >
          <span
            class="theme-preview"
            :class="`pal-${option.id}-${theme.resolvedMode}`"
            aria-hidden="true"
          >
            <span class="tp-card">
              <span class="tp-cover" />
              <span class="tp-text">
                <span class="tp-title" />
                <span class="tp-line" />
                <span class="tp-line short" />
              </span>
            </span>
            <span class="tp-accent" />
          </span>
          <span class="theme-name">
            {{ t(option.labelKey) }}
            <span v-if="theme.theme === option.id" class="badge club-pick">{{ t("common.active") }}</span>
          </span>
          <span class="fine subtle">{{ t(option.blurbKey) }}</span>
        </button>
      </div>
    </section>

    <section class="section" aria-labelledby="import">
      <div class="section-head">
        <h2 id="import">{{ t("settings.importTitle") }}</h2>
      </div>
      <p class="fine muted">
        {{ t("settings.importBlurb") }}
      </p>
      <GoodreadsImport />
    </section>

    <section class="section" aria-labelledby="your-data">
      <div class="section-head">
        <h2 id="your-data">{{ t("settings.yourData") }}</h2>
      </div>
      <p class="fine muted">
        {{ t("settings.exportBlurb") }}
      </p>
      <a class="btn btn-ghost backup-btn" href="/api/auth/me/export" download>{{ t("settings.exportData") }}</a>
    </section>

    <section v-if="session.user?.is_admin" class="section" aria-labelledby="instance-admin">
      <div class="section-head">
        <h2 id="instance-admin">{{ t("admin.title") }}</h2>
      </div>
      <p class="fine muted">{{ t("admin.settingsBlurb") }}</p>
      <AdminLink />
    </section>

    <DeleteAccount @deleted="toLogin(t('account.deleted'))" />

    <hr class="divider" />

    <button class="btn btn-danger" type="button" @click="logout">{{ t("settings.logOut") }}</button>
  </section>
</template>

<style scoped>
.profile-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
  margin: var(--space-3) 0 0;
}

.profile-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.profile-actions label.btn {
  position: relative;
  cursor: pointer;
}

.profile-actions label.btn:focus-within {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.profile-actions label.btn input {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
}

.mode-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-5);
}

.mode-row .segmented {
  flex: 0 1 300px;
}

.theme-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: var(--space-3);
}

.theme-card {
  display: grid;
  gap: var(--space-1);
  text-align: left;
  padding: var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--surface);
  color: inherit;
  transition: border-color var(--dur-fast) var(--ease);
}

.theme-card:hover {
  border-color: var(--border-strong);
}

.theme-card.active {
  border-color: var(--accent);
  box-shadow: inset 0 0 0 1px var(--accent);
}

.theme-name {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 650;
  margin-top: var(--space-2);
}

/* A miniature of the app, painted in the palette the .pal-* class supplies.
   Every value below resolves against that palette, not the active one. */
.theme-preview {
  display: block;
  position: relative;
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: var(--bg);
  border: 1px solid var(--border);
  overflow: hidden;
}

.tp-card {
  display: flex;
  gap: var(--space-2);
  padding: var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--surface);
  border: 1px solid var(--border);
}

.tp-cover {
  flex: 0 0 auto;
  width: 22px;
  height: 33px;
  border-radius: 2px;
  background: var(--surface-3);
}

.tp-text {
  flex: 1;
  display: grid;
  align-content: center;
  gap: 5px;
}

.tp-title {
  height: 7px;
  border-radius: 2px;
  background: var(--text);
}

.tp-line {
  height: 5px;
  border-radius: 2px;
  background: var(--text-muted);
  opacity: 0.55;
}

.tp-line.short {
  width: 55%;
}

.tp-accent {
  display: block;
  width: 38px;
  height: 9px;
  margin-top: var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--accent);
}

.backup-btn {
  margin-top: var(--space-3);
}

@media (min-width: 720px) {
  .backup-btn {
    max-width: 320px;
  }
}
</style>
