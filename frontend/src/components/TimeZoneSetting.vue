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
 * accounts may carry another one, so the device's zone is offered when it differs.
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
    toast.show(t("settings.timezoneSaved", { zone: session.user.timezone }));
  } catch (err) {
    zone.value = saved.value;
    toast.show(err instanceof ApiError ? err.message : t("settings.timezoneFailed"));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="section" aria-labelledby="timezone">
    <div class="section-head">
      <h2 id="timezone">{{ t("settings.timezone") }}</h2>
    </div>
    <p class="fine muted">{{ t("settings.timezoneBlurb") }}</p>
    <label class="field timezone-field">
      <span>{{ t("settings.timezone") }}</span>
      <select :value="zone" name="timezone" :disabled="busy" @change="save(($event.target as HTMLSelectElement).value)">
        <option v-for="option in zones" :key="option" :value="option">{{ option }}</option>
      </select>
    </label>
    <p v-if="device && device !== saved" class="fine subtle device-zone">
      {{ t("settings.deviceZone", { zone: device }) }}
      <button class="text-btn" type="button" :disabled="busy" @click="save(device)">
        {{ t("settings.useDeviceZone", { zone: device }) }}
      </button>
    </p>
  </section>
</template>

<style scoped>
.timezone-field {
  max-width: 22rem;
  margin-top: var(--space-3);
}

.device-zone {
  margin-top: var(--space-2);
}
</style>
