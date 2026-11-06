<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { browserTimeZone, timeZoneOptions } from "../timezone";

/**
 * The personal time zone (`UserOut.timezone`): reading dates, "today" on the
 * shelf, diary and quote times. Sign-up stores the browser's; imported
 * accounts may carry another one, so the device's zone comes first when it
 * differs. Settings shows it in a sheet; the row behind it shows the saved
 * zone, so a save needs no toast.
 */
const { t } = useI18n();
const session = useSession();
const toast = useToast();

const saved = computed(() => session.user?.timezone || "UTC");
const zone = ref(saved.value);
const zones = computed(() => timeZoneOptions(saved.value));
const device = browserTimeZone();
const busy = ref(false);

watch(saved, (value) => {
  zone.value = value;
});

async function save(next: string) {
  if (busy.value || !session.user || next === session.user.timezone) return;
  busy.value = true;
  zone.value = next;
  try {
    session.user = await api.savePreferences({ timezone: next });
  } catch (err) {
    zone.value = saved.value;
    toast.show(err instanceof ApiError ? err.message : t("settings.timezoneFailed"));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="timezone-setting">
    <p v-if="device && device !== saved" class="device-zone">
      <span class="device-name">{{ t("settings.deviceZone", { zone: device }) }}</span>
      <button class="btn btn-primary btn-sm" type="button" :disabled="busy" @click="save(device)">
        {{ t("settings.useDeviceZone") }}
      </button>
    </p>
    <label class="field timezone-field">
      <span>{{ t("settings.timezone") }}</span>
      <select :value="zone" name="timezone" :disabled="busy" @change="save(($event.target as HTMLSelectElement).value)">
        <option v-for="option in zones" :key="option" :value="option">{{ option }}</option>
      </select>
    </label>
    <p class="fine subtle timezone-hint">{{ t("settings.timezoneBlurb") }}</p>
  </div>
</template>

<style scoped>
.device-zone {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2) var(--space-3);
  padding-bottom: var(--space-4);
  margin-bottom: var(--space-4);
  border-bottom: 1px solid var(--border);
}

.device-name {
  min-width: 0;
  overflow-wrap: anywhere;
}

.timezone-hint {
  margin-top: var(--space-2);
}
</style>
