<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import { useToast } from "../../stores/toast";
import type { NotificationsOut } from "../../api/generated";

/**
 * The notification choices (`/api/auth/notifications`). The server stores
 * them, but nothing sends a notification before Phase 4 (#60, Web Push), so
 * Settings does not show this yet; it is kept for that.
 */
const { t } = useI18n();
const toast = useToast();

const notifications = ref<NotificationsOut | null>(null);
const KEYS = ["notify_meeting", "notify_pick", "notify_note"] as const;

async function load() {
  try {
    notifications.value = await api.notifications();
  } catch {
    notifications.value = null;
  }
}

async function toggle(key: keyof NotificationsOut, value: boolean) {
  const before = notifications.value;
  if (!before) return;
  notifications.value = { ...before, [key]: value };
  try {
    notifications.value = await api.saveNotifications({ [key]: value });
    toast.show(t("account.notificationsSaved"));
  } catch (err) {
    notifications.value = before;
    toast.show(err instanceof ApiError ? err.message : t("account.notificationsFailed"));
  }
}

onMounted(load);
</script>

<template>
  <div v-if="notifications" class="notification-list">
    <p class="fine muted">{{ t("account.notificationsBlurb") }}</p>
    <label v-for="key in KEYS" :key="key" class="notification-row">
      <input
        type="checkbox"
        :checked="notifications[key]"
        @change="toggle(key, ($event.target as HTMLInputElement).checked)"
      />
      {{ t(`account.${key}`) }}
    </label>
  </div>
</template>

<style scoped>
.notification-list {
  display: grid;
  gap: var(--space-1);
}

.notification-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--tap);
}
</style>
