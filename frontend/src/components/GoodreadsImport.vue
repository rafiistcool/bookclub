<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { IMPORT_JOB_KEY, importFinished, importOutcome, importPercent, pollImport, skipLine } from "../goodreads";
import { useToast } from "../stores/toast";
import type { GoodreadsImport } from "../types";

const { t } = useI18n();
const toast = useToast();

const file = ref<File | null>(null);
const starting = ref(false);
const job = ref<GoodreadsImport | null>(null);
let polling: AbortController | null = null;

const running = computed(() => Boolean(job.value && !importFinished(job.value)));
const percent = computed(() => (job.value ? importPercent(job.value) : 0));

function remember(id: number | null) {
  try {
    if (id === null) sessionStorage.removeItem(IMPORT_JOB_KEY);
    else sessionStorage.setItem(IMPORT_JOB_KEY, String(id));
  } catch {
    /* Private mode: the import still runs, this tab just forgets it. */
  }
}

function recalled(): number | null {
  try {
    const raw = sessionStorage.getItem(IMPORT_JOB_KEY);
    return raw && /^\d+$/.test(raw) ? Number(raw) : null;
  } catch {
    return null;
  }
}

/** Follow a job until it ends; the server keeps running it if this page goes away. */
async function follow(current: GoodreadsImport, announce: boolean) {
  job.value = current;
  remember(current.id);
  if (importFinished(current)) return;
  polling?.abort();
  const controller = new AbortController();
  polling = controller;
  try {
    const final = await pollImport(
      () => api.importJob(current.id),
      (update) => {
        job.value = update;
      },
      { signal: controller.signal },
    );
    if (final && announce) toast.show(importOutcome(final));
  } catch (err) {
    if (!controller.signal.aborted) {
      toast.show(err instanceof ApiError ? err.message : t("settings.importFailed"));
    }
  }
}

function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  file.value = input.files?.[0] ?? null;
}

async function start() {
  const chosen = file.value;
  if (!chosen || starting.value || running.value) return;
  starting.value = true;
  try {
    const queued = await api.importGoodreads(chosen);
    file.value = null;
    void follow(queued, true);
  } catch (err) {
    // One import at a time: show the one that is already running.
    if (err instanceof ApiError && err.status === 409 && err.job) {
      toast.show(err.message);
      void follow(err.job, true);
    } else {
      toast.show(err instanceof ApiError ? err.message : t("settings.importFailed"));
    }
  } finally {
    starting.value = false;
  }
}

onMounted(async () => {
  const id = recalled();
  if (id === null) return;
  try {
    void follow(await api.importJob(id), true);
  } catch {
    remember(null);
  }
});

onBeforeUnmount(() => polling?.abort());
</script>

<template>
  <div class="goodreads-import">
    <label class="import-file">
      <input type="file" accept=".csv,text/csv" :disabled="running" @change="onFile" />
      <span>{{ file ? file.name : t("settings.chooseCsv") }}</span>
    </label>
    <button
      class="btn btn-ghost"
      type="button"
      :disabled="!file || starting || running"
      @click="start"
    >
      {{ starting || running ? t("settings.importing") : t("settings.importCsv") }}
    </button>

    <template v-if="job">
      <div v-if="running" class="import-progress" aria-live="polite">
        <span
          class="progress-track"
          role="progressbar"
          :aria-valuenow="percent"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-label="t('settings.importProgressLabel')"
        >
          <span class="progress-fill" :style="{ width: `${percent}%` }" />
        </span>
        <p class="fine subtle nums">
          {{ job.status === "queued" ? t("settings.importQueued") : t("settings.importProgress", { processed: job.processed, total: job.total }) }}
        </p>
        <p class="fine subtle">{{ t("settings.importWait") }}</p>
      </div>
      <p v-else-if="job.status === 'failed'" class="error import-summary">
        {{ importOutcome(job) }}
      </p>
      <p v-if="job.status !== 'queued'" class="fine muted import-summary">
        {{ t("settings.importSummary", { imported: job.imported, skipped: job.skipped }) }}
      </p>
      <ul v-if="job.skips.length" class="import-skips">
        <li v-for="(skip, index) in job.skips" :key="index">{{ skipLine(skip) }}</li>
      </ul>
    </template>
  </div>
</template>

<style scoped>
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

.import-progress {
  display: grid;
  gap: var(--space-1);
  margin-top: var(--space-3);
}

.progress-track {
  display: block;
  height: 6px;
  max-width: 320px;
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  overflow: hidden;
}

.progress-fill {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width var(--dur-base) var(--ease);
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

@media (min-width: 720px) {
  .import-file {
    max-width: 320px;
  }
}
</style>
