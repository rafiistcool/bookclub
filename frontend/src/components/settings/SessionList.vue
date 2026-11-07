<script setup lang="ts">
import { ComputerDesktopIcon, DevicePhoneMobileIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import { describeUserAgent } from "../../devices";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import type { SessionOut } from "../../api/generated";
import RelativeTime from "../RelativeTime.vue";

/**
 * The signed-in devices, this one first, each with "Sign out"; ending this
 * one leaves the app. "Last used" is relative, with the exact time as its
 * title, so each row's second line stays one line.
 */
const emit = defineEmits<{ loggedOut: [] }>();

const { t } = useI18n();
const session = useSession();
const toast = useToast();
const zone = computed(() => session.user?.timezone || "UTC");

const sessions = ref<SessionOut[]>([]);
const loaded = ref(false);
const error = ref("");
const ending = ref<number | null>(null);

function message(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    // This device first; the others keep the server's order (stable sort).
    sessions.value = [...(await api.sessions()).items].sort((a, b) => Number(b.current) - Number(a.current));
    error.value = "";
  } catch (err) {
    error.value = message(err, t("account.sessionsFailed"));
  } finally {
    loaded.value = true;
  }
}

async function end(row: SessionOut) {
  if (ending.value !== null) return;
  ending.value = row.id;
  try {
    await api.endSession(row.id);
    if (row.current) {
      session.reset({ leavingDevice: true });
      emit("loggedOut");
      return;
    }
    sessions.value = sessions.value.filter((item) => item.id !== row.id);
    toast.show(t("account.sessionEnded"));
  } catch (err) {
    toast.show(message(err, t("account.sessionsFailed")));
  } finally {
    ending.value = null;
  }
}

/** "Firefox on Linux" for a browser; an app's own label; the raw User-Agent when it says too little. */
function deviceName(row: { kind: string; label: string }): string {
  if (row.kind === "bearer") return row.label || t("account.app");
  if (!row.label) return t("account.browser");
  const { browser, os } = describeUserAgent(row.label);
  if (browser && os) return t("account.deviceOn", { browser, os });
  return browser ?? row.label;
}

/** A phone for the apps and for a mobile browser, a screen otherwise. */
function deviceIcon(row: { kind: string; label: string }) {
  if (row.kind === "bearer") return DevicePhoneMobileIcon;
  return /Android|iPhone|iPad|Mobile/i.test(row.label) ? DevicePhoneMobileIcon : ComputerDesktopIcon;
}

onMounted(load);
</script>

<template>
  <p v-if="error" class="error">{{ error }}</p>
  <ul v-else-if="!loaded" class="session-list sessions-skeleton" aria-hidden="true">
    <li v-for="n in 3" :key="n" class="session-row">
      <span class="skeleton skeleton-icon" />
      <span class="session-meta">
        <span class="skeleton skeleton-line wide" />
        <span class="skeleton skeleton-line narrow" />
      </span>
    </li>
  </ul>
  <ul v-else class="session-list">
    <li v-for="row in sessions" :key="row.id" class="session-row">
      <component :is="deviceIcon(row)" class="session-icon" aria-hidden="true" />
      <span class="session-meta">
        <span class="session-label" :title="row.label || undefined">{{ deviceName(row) }}</span>
        <span class="finer subtle">
          <template v-if="row.current"><span class="session-here">{{ t("account.thisDevice") }}</span> · </template>
          <i18n-t keypath="account.lastUsed" scope="global">
            <template #when><RelativeTime :iso="row.last_used_at" :zone="zone" /></template>
          </i18n-t>
        </span>
      </span>
      <button class="text-btn session-end" type="button" :disabled="ending !== null" @click="end(row)">
        {{ row.current ? t("account.signOutHere") : t("account.signOut") }}
      </button>
    </li>
  </ul>
</template>

<style scoped>
.skeleton-icon {
  flex: none;
  width: 22px;
  height: 22px;
  border-radius: 50%;
}

.sessions-skeleton .session-meta {
  gap: var(--space-2);
}

.sessions-skeleton .skeleton-line {
  margin-top: 0;
}

.skeleton-line.wide {
  width: 60%;
}

.skeleton-line.narrow {
  width: 40%;
}

.session-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.session-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 60px;
  padding: var(--space-2) var(--space-1);
}

.session-row + .session-row {
  border-top: 1px solid var(--border);
}

.session-icon {
  flex: none;
  width: 22px;
  height: 22px;
  color: var(--text-muted);
}

.session-meta {
  flex: 1;
  display: grid;
  gap: 2px;
  min-width: 0;
}

.session-label {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.session-here {
  color: var(--accent);
  font-weight: 600;
}

.session-end {
  flex: none;
  min-height: var(--tap);
}
</style>
