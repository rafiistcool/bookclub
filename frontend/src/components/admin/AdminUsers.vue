<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { AdminRequestSignalOut, AdminUserOut } from "../../api/generated";
import { formatClubDate } from "../../i18n/dates";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import Avatar from "../Avatar.vue";
import AdminDisclosure from "./AdminDisclosure.vue";

/**
 * Suspended accounts (#55, Q15): suspend one by its username, and lift a
 * suspension from the list. A suspended account cannot sign in and loses
 * every session and app token at once; what it wrote stays. Suspending needs
 * a second tap. The server refuses the admin's own account and names an
 * unknown one (`errors.cannotSuspendSelf`, `errors.noAccount`).
 *
 * The second view lists the accounts whose friend requests may be spam (REQS,
 * #55): pending requests out and blocks received in the last 30 days, counts
 * only. "Suspend…" on a row puts its name in the form above, at the second
 * tap, so the suspension works and confirms as for a typed name.
 */
const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const items = ref<AdminUserOut[]>([]);
const loaded = ref(false);
const error = ref("");
const name = ref("");
/** The name the next tap suspends; set by the first tap. */
const confirming = ref<string | null>(null);
const problem = ref("");
const busy = ref(false);
/** The username whose suspension is being lifted. */
const lifting = ref<string | null>(null);

/** The list under the form: suspended accounts, or accounts by their friend requests. */
const view = ref<"suspended" | "requests">("suspended");
const signals = ref<AdminRequestSignalOut[]>([]);
const signalsLoaded = ref(false);
const signalsError = ref("");
const confirmButton = ref<HTMLButtonElement | null>(null);

const typed = computed(() => name.value.trim());

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    items.value = (await api.adminSuspendedUsers()).items;
    error.value = "";
  } catch (err) {
    error.value = failed(err, t("admin.loadFailed"));
  } finally {
    loaded.value = true;
  }
}

async function loadSignals() {
  try {
    signals.value = (await api.adminFriendRequestSignals()).items;
    signalsError.value = "";
  } catch (err) {
    signalsError.value = failed(err, t("admin.loadFailed"));
  } finally {
    signalsLoaded.value = true;
  }
}

async function showView(next: "suspended" | "requests") {
  if (view.value === next) return;
  view.value = next;
  if (next === "requests") await loadSignals();
}

/** "Suspend…" on a row: the form above, at its second tap, for that name. */
async function prepareSuspension(username: string) {
  name.value = username;
  confirming.value = username;
  problem.value = "";
  await nextTick();
  confirmButton.value?.focus();
  confirmButton.value?.scrollIntoView?.({ block: "center" });
}

/** Any change to the name asks for a new first tap. */
function edited() {
  confirming.value = null;
  problem.value = "";
}

async function suspend() {
  if (!typed.value || busy.value) return;
  if (confirming.value !== typed.value) {
    confirming.value = typed.value;
    problem.value = "";
    return;
  }
  busy.value = true;
  try {
    const user = await api.adminSuspendUser(typed.value);
    items.value = [user, ...items.value.filter((row) => row.id !== user.id)];
    signals.value = signals.value.map((row) => (row.user.id === user.id ? { ...row, user } : row));
    toast.show(t("admin.users.suspended", { name: user.username }));
    name.value = "";
    confirming.value = null;
  } catch (err) {
    problem.value = failed(err, t("admin.users.failed"));
    confirming.value = null;
  } finally {
    busy.value = false;
  }
}

async function unsuspend(user: AdminUserOut) {
  if (lifting.value) return;
  lifting.value = user.username;
  try {
    await api.adminUnsuspendUser(user.username);
    items.value = items.value.filter((row) => row.id !== user.id);
    signals.value = signals.value.map((row) =>
      row.user.id === user.id ? { ...row, user: { ...row.user, suspended_at: null } } : row,
    );
    toast.show(t("admin.users.unsuspended", { name: user.username }));
  } catch (err) {
    toast.show(failed(err, t("admin.users.failed")));
  } finally {
    lifting.value = null;
  }
}

function since(user: AdminUserOut): string {
  const when = formatClubDate(user.suspended_at, session.user?.timezone || "UTC", locale.locale);
  return t("admin.users.since", { when });
}

onMounted(load);
</script>

<template>
  <section class="admin-queue" aria-labelledby="admin-users">
    <h2 id="admin-users" class="visually-hidden">{{ t("admin.users.title") }}</h2>
    <p class="fine muted">{{ t("admin.users.blurb") }}</p>
    <AdminDisclosure class="admin-howto" :summary="t('admin.howTo')">
      <p class="fine muted">{{ t("admin.users.howTo") }}</p>
    </AdminDisclosure>

    <form class="admin-form suspend-form" @submit.prevent="suspend">
      <label class="field">
        <span>{{ t("admin.users.name") }}</span>
        <input
          v-model="name"
          name="suspend-username"
          type="text"
          autocomplete="off"
          autocapitalize="none"
          spellcheck="false"
          required
          @input="edited"
        />
      </label>
      <p v-if="confirming" class="fine">{{ t("admin.users.confirmBlurb", { name: confirming }) }}</p>
      <p v-if="problem" class="error fine" role="alert">{{ problem }}</p>
      <div class="btn-row">
        <button ref="confirmButton" class="btn btn-danger btn-sm" type="submit" :disabled="busy || !typed">
          {{ confirming ? t("admin.users.suspendConfirm", { name: confirming }) : t("admin.users.suspend") }}
        </button>
        <button v-if="confirming" class="btn btn-ghost btn-sm" type="button" @click="confirming = null">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>

    <div class="segmented user-views" role="group" :aria-label="t('admin.users.views')">
      <button type="button" :aria-pressed="view === 'suspended'" @click="showView('suspended')">
        {{ t("admin.users.suspendedView") }}
      </button>
      <button type="button" :aria-pressed="view === 'requests'" @click="showView('requests')">
        {{ t("admin.users.requests.view") }}
      </button>
    </div>

    <template v-if="view === 'requests'">
      <p class="fine muted section-blurb">{{ t("admin.users.requests.blurb") }}</p>
      <p v-if="signalsError" class="error">
        {{ signalsError }}
        <button class="text-btn" type="button" @click="loadSignals">{{ t("common.tryAgain") }}</button>
      </p>
      <ol v-else-if="!signalsLoaded" class="skeleton-rows admin-skeleton" aria-hidden="true">
        <li v-for="n in 2" :key="n" class="skeleton-row">
          <span class="skeleton skeleton-avatar" />
          <span class="skeleton-row-lines">
            <span class="skeleton skeleton-line short" />
            <span class="skeleton skeleton-line" />
          </span>
        </li>
      </ol>
      <p v-else-if="signals.length === 0" class="fine subtle">{{ t("admin.users.requests.none") }}</p>
      <ul v-else class="user-list signal-list">
        <li v-for="row in signals" :key="row.user.id" class="user-row signal-row">
          <Avatar :username="row.user.username" :src="row.user.avatar_url" size="sm" />
          <span class="user-meta">
            <strong>{{ row.user.username }}</strong>
            <span class="finer subtle nums">
              {{ t("admin.users.requests.counts", { pending: row.pending_requests, blocks: row.blocks_received }) }}
            </span>
          </span>
          <span v-if="row.user.suspended_at" class="badge">{{ t("admin.users.requests.suspended") }}</span>
          <button
            v-else
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="busy || row.user.username === session.user?.username"
            @click="prepareSuspension(row.user.username)"
          >
            {{ t("admin.users.requests.suspend") }}
          </button>
        </li>
      </ul>
    </template>

    <template v-else>
      <p v-if="error" class="error">
        {{ error }}
        <button class="text-btn" type="button" @click="load">{{ t("common.tryAgain") }}</button>
      </p>
      <ol v-else-if="!loaded" class="skeleton-rows admin-skeleton" aria-hidden="true">
        <li v-for="n in 2" :key="n" class="skeleton-row">
          <span class="skeleton skeleton-avatar" />
          <span class="skeleton-row-lines">
            <span class="skeleton skeleton-line short" />
            <span class="skeleton skeleton-line" />
          </span>
        </li>
      </ol>
      <p v-else-if="items.length === 0" class="fine subtle">{{ t("admin.users.none") }}</p>
      <ul v-else class="user-list">
        <li v-for="user in items" :key="user.id" class="user-row">
          <Avatar :username="user.username" :src="user.avatar_url" size="sm" />
          <span class="user-meta">
            <strong>{{ user.username }}</strong>
            <span class="finer subtle">
              <span v-if="user.is_admin" class="badge">{{ t("admin.users.admin") }}</span>
              {{ since(user) }}
            </span>
          </span>
          <button class="btn btn-ghost btn-sm" type="button" :disabled="lifting === user.username" @click="unsuspend(user)">
            {{ t("admin.users.unsuspend") }}
          </button>
        </li>
      </ul>
    </template>
  </section>
</template>

<style scoped>
.skeleton-avatar {
  flex: none;
  width: 24px;
  height: 24px;
  border-radius: 50%;
}

.suspend-form input {
  max-width: 22rem;
}

.user-views {
  max-width: 26rem;
  margin: var(--space-3) 0;
}

.section-blurb {
  margin: 0;
}

/* Light rows split by hairlines, as in the queues and Settings. */
.user-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.user-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 60px;
  padding: var(--space-2) 0;
}

.user-row + .user-row {
  border-top: 1px solid var(--border);
}

.user-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.user-meta strong {
  overflow-wrap: anywhere;
}
</style>
