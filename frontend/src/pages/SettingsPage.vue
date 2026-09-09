<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import { MODES, THEMES, useTheme } from "../stores/theme";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { GoodreadsImport, Invite } from "../types";

const { t } = useI18n();
const router = useRouter();
const theme = useTheme();
const session = useSession();
const toast = useToast();
const avatarBusy = ref(false);

const invites = ref<Invite[]>([]);
const invitesError = ref("");
const invitesLoaded = ref(false);
const minting = ref(false);

const importFile = ref<File | null>(null);
const importing = ref(false);
const importResult = ref<GoodreadsImport | null>(null);

const username = computed(() => session.user?.username ?? "");
const unusedInvites = computed(() => invites.value.filter((row) => !row.used).length);

async function loadInvites() {
  try {
    invites.value = await api.invites();
    invitesError.value = "";
  } catch (err) {
    invitesError.value =
      err instanceof ApiError ? err.message : t("settings.invitesLoadFailed");
  } finally {
    invitesLoaded.value = true;
  }
}

async function mint() {
  minting.value = true;
  try {
    const created = await api.createInvite();
    await loadInvites();
    await copy(created.code);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("settings.inviteFailed"));
  } finally {
    minting.value = false;
  }
}

async function copy(code: string) {
  try {
    await navigator.clipboard.writeText(code);
    toast.show(t("settings.inviteCopied"));
  } catch {
    toast.show(code);
  }
}

function onImportFile(event: Event) {
  const input = event.target as HTMLInputElement;
  importFile.value = input.files?.[0] ?? null;
  importResult.value = null;
}

async function importCsv() {
  const file = importFile.value;
  if (!file || importing.value) return;
  importing.value = true;
  try {
    const result = await api.importGoodreads(file);
    importResult.value = result;
    if (result.imported) {
      toast.show(
        result.skipped
          ? t("settings.importedSkipped", {
              imported: result.imported,
              skipped: result.skipped,
            })
          : t("settings.importedBooks", { n: result.imported }),
      );
    } else {
      toast.show(
        result.skipped ? t("settings.skippedOnly", { n: result.skipped }) : t("settings.nothingToImport"),
      );
    }
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("settings.importFailed"));
  } finally {
    importing.value = false;
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

onMounted(loadInvites);
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

    <section class="section" aria-labelledby="language">
      <div class="section-head">
        <h2 id="language">{{ t("settings.language") }}</h2>
      </div>
      <p class="fine muted">{{ t("settings.languageBlurb") }}</p>
      <div class="mode-row">
        <LanguageSwitch />
      </div>
    </section>

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

    <section class="section" aria-labelledby="invites">
      <div class="section-head">
        <h2 id="invites">{{ t("settings.invites") }}</h2>
        <span class="fine subtle nums">{{ t("settings.unusedCount", { n: unusedInvites }) }}</span>
      </div>
      <p class="fine muted">
        {{ t("settings.invitesBlurb") }}
      </p>
      <button
        class="btn btn-primary mint-btn"
        type="button"
        :disabled="minting"
        @click="mint"
      >
        {{ minting ? t("settings.creatingInvite") : t("settings.createInvite") }}
      </button>
      <p v-if="invitesError" class="error">{{ invitesError }}</p>
      <ul v-else-if="invites.length" class="invite-list">
        <li v-for="invite in invites" :key="invite.code" class="invite-row">
          <span>
            <code class="invite-code">{{ invite.code }}</code>
            <span class="fine subtle invite-state">
              <template v-if="invite.used">{{ t("settings.usedBy", { name: invite.used_by }) }}</template>
              <template v-else>{{ t("settings.unused") }}</template>
            </span>
          </span>
          <button
            v-if="!invite.used"
            class="btn btn-ghost btn-sm"
            type="button"
            @click="copy(invite.code)"
          >
            {{ t("common.copy") }}
          </button>
        </li>
      </ul>
      <ul v-else-if="!invitesLoaded" class="invite-list" aria-hidden="true">
        <li v-for="n in 2" :key="n" class="invite-row">
          <span class="skeleton skeleton-line" />
        </li>
      </ul>
      <p v-else class="fine subtle">{{ t("settings.noInvites") }}</p>
    </section>

    <section class="section" aria-labelledby="import">
      <div class="section-head">
        <h2 id="import">{{ t("settings.importTitle") }}</h2>
      </div>
      <p class="fine muted">
        {{ t("settings.importBlurb") }}
      </p>
      <label class="import-file">
        <input type="file" accept=".csv,text/csv" @change="onImportFile" />
        <span>{{ importFile ? importFile.name : t("settings.chooseCsv") }}</span>
      </label>
      <button
        class="btn btn-ghost"
        type="button"
        :disabled="!importFile || importing"
        @click="importCsv"
      >
        {{ importing ? t("settings.importing") : t("settings.importCsv") }}
      </button>
      <p v-if="importing" class="fine subtle">
        {{ t("settings.importWait") }}
      </p>
      <template v-if="importResult">
        <p class="fine muted import-summary">
          {{ t("settings.importSummary", { imported: importResult.imported, skipped: importResult.skipped }) }}
        </p>
        <ul v-if="importResult.skips.length" class="import-skips">
          <li v-for="(skip, index) in importResult.skips" :key="index">
            {{ skip.title || "Untitled" }} — {{ skip.reason }}
          </li>
        </ul>
      </template>
    </section>

    <section class="section" aria-labelledby="backup">
      <div class="section-head">
        <h2 id="backup">{{ t("settings.backup") }}</h2>
      </div>
      <p class="fine muted">
        {{ t("settings.backupBlurb") }}
      </p>
      <a class="btn btn-ghost backup-btn" href="/api/backup">{{ t("settings.downloadDatabase") }}</a>
    </section>

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

.mint-btn {
  margin: var(--space-3) 0;
}

.invite-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.invite-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.invite-code {
  font-family: var(--mono);
  letter-spacing: 0.08em;
  display: block;
}

.invite-state {
  display: block;
}

.import-file {
  position: relative;
  display: block;
  width: 100%;
  min-height: var(--tap);
  padding: var(--space-3);
  margin: var(--space-3) 0;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--text-muted);
  cursor: pointer;
}

.import-file:hover {
  border-color: var(--accent);
}

.import-file input {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
}

.import-summary {
  margin-top: var(--space-3);
}

.import-skips {
  margin: var(--space-2) 0 0;
  padding-left: var(--space-5);
  color: var(--text-subtle);
  font-size: var(--text-sm);
}

.backup-btn {
  margin-top: var(--space-3);
}

@media (min-width: 720px) {
  .mint-btn,
  .backup-btn,
  .import-file {
    max-width: 320px;
  }
}
</style>
