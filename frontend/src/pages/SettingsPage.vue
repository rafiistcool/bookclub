<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import NavIcon from "../components/NavIcon.vue";
import { usePush } from "../stores/push";
import { useSession } from "../stores/session";
import { useShelf } from "../stores/shelf";
import { useTheme, type ThemePref } from "../stores/theme";
import { useToast } from "../stores/toast";
import type { GoodreadsImport, Invite, NotificationPrefs } from "../types";

const session = useSession();
const theme = useTheme();
const push = usePush();
const shelf = useShelf();
const toast = useToast();
const router = useRouter();
const route = useRoute();

// --- invites ---
const invites = ref<Invite[]>([]);
const minting = ref(false);

async function loadInvites() {
  try {
    invites.value = await api.invites();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not load invites");
  }
}

async function mint() {
  minting.value = true;
  try {
    const created = await api.createInvite();
    await loadInvites();
    await copy(created.code);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not create an invite");
  } finally {
    minting.value = false;
  }
}

async function copy(code: string) {
  try {
    await navigator.clipboard.writeText(code);
    toast.show("Invite copied");
  } catch {
    toast.show(code);
  }
}

const unusedInvites = computed(() => invites.value.filter((row) => !row.used));
const usedInvites = computed(() => invites.value.filter((row) => row.used));

// --- notifications ---
const prefs = ref<NotificationPrefs | null>(null);

async function loadPrefs() {
  try {
    prefs.value = await api.notificationPrefs();
  } catch {
    prefs.value = null;
  }
}

async function togglePref(key: keyof NotificationPrefs) {
  if (!prefs.value) return;
  const next = !prefs.value[key];
  prefs.value = { ...prefs.value, [key]: next };
  try {
    prefs.value = await api.updateNotificationPrefs({ [key]: next });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save");
    await loadPrefs();
  }
}

async function togglePush() {
  if (push.subscribed) await push.unsubscribe();
  else await push.subscribe();
  if (push.error) toast.show(push.error);
  else toast.show(push.subscribed ? "Notifications on for this device" : "Notifications off for this device");
}

async function testPush() {
  try {
    await api.testPush();
    toast.show("Test sent — it should arrive in a moment");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not send a test");
  }
}

// --- password ---
const currentPassword = ref("");
const newPassword = ref("");
const showPasswords = ref(false);
const passwordPending = ref(false);
const passwordError = ref("");

async function changePassword() {
  passwordError.value = "";
  passwordPending.value = true;
  try {
    await api.changePassword({ current_password: currentPassword.value, new_password: newPassword.value });
    currentPassword.value = "";
    newPassword.value = "";
    toast.show("Password changed");
  } catch (err) {
    passwordError.value = err instanceof ApiError ? err.message : "Could not change the password";
  } finally {
    passwordPending.value = false;
  }
}

// --- import ---
const importFile = ref<File | null>(null);
const importing = ref(false);
const importResult = ref<GoodreadsImport | null>(null);

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
    await shelf.refresh();
    toast.show(result.imported ? `Imported ${result.imported}` : "Nothing to import");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not import that file");
  } finally {
    importing.value = false;
  }
}

async function logout() {
  await session.logout();
  await router.push("/login");
}

const THEMES: { value: ThemePref; label: string; icon: "sun" | "moon" | "check" }[] = [
  { value: "system", label: "System", icon: "check" },
  { value: "light", label: "Light", icon: "sun" },
  { value: "dark", label: "Dark", icon: "moon" },
];

onMounted(async () => {
  await Promise.all([loadInvites(), loadPrefs(), push.init()]);
  if (route.hash) {
    document.querySelector(route.hash)?.scrollIntoView({ block: "start" });
  }
});
</script>

<template>
  <section aria-label="Settings">
    <header style="display: flex; align-items: center; gap: 12px; margin-bottom: 8px">
      <Avatar :username="session.user?.username ?? '?'" size="lg" />
      <div style="flex: 1">
        <h1 style="font-size: var(--text-xl)">{{ session.user?.username }}</h1>
        <p class="fine muted">Member</p>
      </div>
      <button class="btn btn-ghost btn-sm" type="button" @click="logout">Log out</button>
    </header>

    <section class="settings-group" aria-labelledby="theme-title">
      <h2 id="theme-title">Appearance</h2>
      <div class="segmented" role="radiogroup" aria-label="Theme">
        <button
          v-for="option in THEMES"
          :key="option.value"
          type="button"
          role="radio"
          :aria-checked="theme.pref === option.value"
          :aria-selected="theme.pref === option.value"
          @click="theme.set(option.value)"
        >
          <span style="display: inline-flex; gap: 6px; align-items: center">
            <NavIcon v-if="option.icon !== 'check'" :name="option.icon" :size="16" />
            {{ option.label }}
          </span>
        </button>
      </div>
    </section>

    <section id="notifications" class="settings-group" aria-labelledby="notify-title">
      <h2 id="notify-title">Notifications</h2>
      <div class="card">
        <div class="toggle-row">
          <div>
            <p class="strong">Push on this device</p>
            <p class="fine muted">
              <template v-if="!push.supported">Not supported in this browser. On iPhone, add the app to your Home Screen first.</template>
              <template v-else-if="push.permission === 'denied'">Blocked in browser settings.</template>
              <template v-else>New picks, notes, and a reminder the day before a meeting.</template>
            </p>
          </div>
          <button
            class="switch"
            type="button"
            role="switch"
            :aria-checked="push.subscribed"
            :disabled="!push.supported || push.busy || push.permission === 'denied'"
            @click="togglePush"
          />
        </div>
        <template v-if="prefs">
          <div class="toggle-row">
            <span>Meeting reminders</span>
            <button class="switch" type="button" role="switch" :aria-checked="prefs.notify_meeting" @click="togglePref('notify_meeting')" />
          </div>
          <div class="toggle-row">
            <span>New club pick</span>
            <button class="switch" type="button" role="switch" :aria-checked="prefs.notify_pick" @click="togglePref('notify_pick')" />
          </div>
          <div class="toggle-row">
            <span>New notes</span>
            <button class="switch" type="button" role="switch" :aria-checked="prefs.notify_note" @click="togglePref('notify_note')" />
          </div>
        </template>
        <button v-if="push.subscribed" class="text-btn sm" type="button" style="margin-top: 6px" @click="testPush">Send a test</button>
      </div>
    </section>

    <section id="invites" class="settings-group" aria-labelledby="invites-title">
      <h2 id="invites-title">Invites</h2>
      <p class="fine muted" style="margin-bottom: 10px">Anyone with a code can join. Treat unused codes like passwords.</p>
      <button class="btn btn-primary" type="button" :disabled="minting" @click="mint">
        <NavIcon name="plus" :size="18" /> {{ minting ? "Creating…" : "Create invite" }}
      </button>
      <div v-if="unusedInvites.length" class="list" style="margin-top: 12px">
        <div v-for="invite in unusedInvites" :key="invite.code" class="row-item">
          <span />
          <div class="row-body">
            <div class="invite-code">{{ invite.code }}</div>
            <p>Unused</p>
          </div>
          <button class="btn btn-ghost btn-sm" type="button" @click="copy(invite.code)">Copy</button>
        </div>
      </div>
      <details v-if="usedInvites.length" style="margin-top: 10px">
        <summary class="fine muted" style="cursor: pointer">{{ usedInvites.length }} used</summary>
        <ul class="fine muted" style="margin: 8px 0 0; padding-left: 18px">
          <li v-for="invite in usedInvites" :key="invite.code"><span class="invite-code">{{ invite.code }}</span> · {{ invite.used_by }}</li>
        </ul>
      </details>
    </section>

    <section id="password" class="settings-group" aria-labelledby="password-title">
      <h2 id="password-title">Password</h2>
      <form class="card" @submit.prevent="changePassword">
        <p v-if="passwordError" class="error fine" style="margin-bottom: 10px">{{ passwordError }}</p>
        <label class="field">
          <span>Current password</span>
          <input v-model="currentPassword" :type="showPasswords ? 'text' : 'password'" autocomplete="current-password" required />
        </label>
        <label class="field">
          <span>New password</span>
          <input v-model="newPassword" :type="showPasswords ? 'text' : 'password'" autocomplete="new-password" minlength="8" required />
          <span class="field-hint">At least 8 characters.</span>
        </label>
        <div class="actions">
          <button class="btn btn-primary" type="submit" :disabled="passwordPending || newPassword.length < 8 || !currentPassword">
            {{ passwordPending ? "Saving…" : "Change password" }}
          </button>
          <button class="text-btn sm" type="button" @click="showPasswords = !showPasswords">{{ showPasswords ? "Hide" : "Show" }}</button>
        </div>
      </form>
    </section>

    <section id="import" class="settings-group" aria-labelledby="import-title">
      <h2 id="import-title">Import from Goodreads</h2>
      <p class="fine muted" style="margin-bottom: 10px">
        Upload your Goodreads library export CSV. Exclusive shelves map to Want to read, Reading, and Finished.
      </p>
      <label class="import-file">
        <input type="file" accept=".csv,text/csv" @change="onImportFile" />
        <span>{{ importFile ? importFile.name : "Choose CSV…" }}</span>
      </label>
      <button class="btn btn-ghost" type="button" style="margin-top: 10px" :disabled="!importFile || importing" @click="importCsv">
        {{ importing ? "Importing…" : "Import" }}
      </button>
      <p v-if="importResult" class="fine muted" style="margin-top: 10px">
        Imported {{ importResult.imported }}. Skipped {{ importResult.skipped }}.
      </p>
      <ul v-if="importResult?.skips.length" class="fine muted" style="margin: 8px 0 0; padding-left: 18px">
        <li v-for="(skip, index) in importResult.skips" :key="index">{{ skip.title || "Untitled" }} — {{ skip.reason }}</li>
      </ul>
    </section>

    <section id="backup" class="settings-group" aria-labelledby="backup-title">
      <h2 id="backup-title">Backup</h2>
      <p class="fine muted" style="margin-bottom: 10px">Download a copy of the club’s SQLite database. Restoring means replacing the file on the server.</p>
      <a class="btn btn-ghost" href="/api/backup">Download database</a>
    </section>
  </section>
</template>
