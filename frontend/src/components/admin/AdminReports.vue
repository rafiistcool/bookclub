<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { ReportOut } from "../../api/generated";
import { bookPath } from "../../constants";
import { formatClubDate } from "../../i18n/dates";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";

/**
 * Notice and takedown: readers' reports of a cover. "Take down" removes the
 * picture everywhere for good (no refresh brings it back); "Keep it" closes
 * the report. Either decision closes every open report of the picture.
 */
const emit = defineEmits<{ count: [n: number] }>();

const REASONS = ["wrong_book", "offensive", "copyright", "other"];

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const items = ref<ReportOut[]>([]);
const loaded = ref(false);
const error = ref("");
const busyId = ref<number | null>(null);
/** Taking a picture down needs a second tap. */
const confirming = ref<number | null>(null);

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    items.value = (await api.adminReports()).items;
    error.value = "";
    emit("count", items.value.length);
  } catch (err) {
    error.value = failed(err, t("admin.loadFailed"));
  } finally {
    loaded.value = true;
  }
}

function drop(id: number) {
  items.value = items.value.filter((row) => row.id !== id);
  emit("count", items.value.length);
}

/** The report and every other open report of its picture, which the decision closed too. */
function dropPicture(report: ReportOut) {
  const picture = report.media_public_id;
  items.value = items.value.filter((row) => row.id !== report.id && (picture == null || row.media_public_id !== picture));
  emit("count", items.value.length);
}

async function resolve(report: ReportOut, action: "remove" | "dismiss") {
  if (busyId.value) return;
  if (action === "remove" && confirming.value !== report.id) {
    confirming.value = report.id;
    return;
  }
  confirming.value = null;
  busyId.value = report.id;
  try {
    await api.resolveReport(report.id, action);
    dropPicture(report);
    toast.show(action === "remove" ? t("admin.takenDown") : t("admin.kept"));
  } catch (err) {
    // Resolved meanwhile (409): it leaves the open queue either way.
    if (err instanceof ApiError && err.status === 409) drop(report.id);
    toast.show(failed(err, t("admin.decideFailed")));
  } finally {
    busyId.value = null;
  }
}

function reason(code: string): string {
  return REASONS.includes(code) ? t(`cover.reasons.${code}`) : code;
}

function when(iso: string): string {
  return formatClubDate(iso, session.user?.timezone || "UTC", locale.locale);
}

onMounted(load);
</script>

<template>
  <section class="admin-queue" aria-labelledby="admin-reports">
    <div class="section-head">
      <h2 id="admin-reports">{{ t("admin.reports") }}</h2>
      <span v-if="items.length" class="fine subtle nums">{{ items.length }}</span>
    </div>
    <p class="fine muted">{{ t("admin.reportsBlurb") }}</p>
    <p v-if="error" class="error">
      {{ error }}
      <button class="text-btn" type="button" @click="load">{{ t("common.tryAgain") }}</button>
    </p>
    <div v-else-if="!loaded" class="skeleton admin-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0" class="fine subtle">{{ t("admin.noReports") }}</p>
    <ol v-else class="admin-list">
      <li v-for="report in items" :key="report.id" class="panel admin-item report-item">
        <p class="admin-item-head fine">
          <span class="badge">{{ reason(report.reason) }}</span>
          <span class="subtle">
            {{ t("admin.byOn", { name: report.created_by?.username ?? t("admin.someone"), when: when(report.created_at) }) }}
          </span>
        </p>
        <div class="report-body">
          <BookCover
            v-if="report.cover_url || report.cover_thumb_url"
            :book="{ title: report.book?.title ?? '', cover_url: report.cover_url, cover_thumb_url: report.cover_thumb_url }"
            size="md"
          />
          <div class="report-text">
            <RouterLink v-if="report.book" :to="bookPath(report.book)">{{ report.book.title }}</RouterLink>
            <p v-if="report.removed" class="fine subtle">{{ t("admin.alreadyRemoved") }}</p>
            <p v-if="report.details" class="fine report-details">{{ report.details }}</p>
          </div>
        </div>
        <p v-if="confirming === report.id" class="fine">{{ t("admin.takeDownBlurb") }}</p>
        <div class="btn-row">
          <button class="btn btn-danger btn-sm" type="button" :disabled="busyId === report.id" @click="resolve(report, 'remove')">
            {{ confirming === report.id ? t("admin.takeDownConfirm") : t("admin.takeDown") }}
          </button>
          <button class="btn btn-ghost btn-sm" type="button" :disabled="busyId === report.id" @click="resolve(report, 'dismiss')">
            {{ t("admin.keep") }}
          </button>
          <button v-if="confirming === report.id" class="btn btn-ghost btn-sm" type="button" @click="confirming = null">
            {{ t("common.cancel") }}
          </button>
        </div>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.admin-skeleton {
  height: 160px;
}

.admin-list {
  list-style: none;
  margin: var(--space-3) 0 0;
  padding: 0;
  display: grid;
  gap: var(--space-3);
}

.admin-item {
  display: grid;
  gap: var(--space-3);
}

.admin-item-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}

.report-body {
  display: flex;
  align-items: flex-start;
  gap: var(--space-4);
}

.report-text {
  display: grid;
  gap: var(--space-1);
  min-width: 0;
}

.report-text p {
  margin: 0;
}

.report-details {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
