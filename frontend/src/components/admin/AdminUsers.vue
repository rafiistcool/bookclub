<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { AdminUserOut } from "../../api/generated";
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
        <button class="btn btn-danger btn-sm" type="submit" :disabled="busy || !typed">
          {{ confirming ? t("admin.users.suspendConfirm", { name: confirming }) : t("admin.users.suspend") }}
        </button>
        <button v-if="confirming" class="btn btn-ghost btn-sm" type="button" @click="confirming = null">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>

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
