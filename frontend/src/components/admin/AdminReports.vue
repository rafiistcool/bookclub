<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { AdminReportStats, ReportOut } from "../../api/generated";
import { formatAge, hoursBetween, isOverdue } from "../../admin";
import { bookPath } from "../../constants";
import { formatClubDate } from "../../i18n/dates";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";
import AdminTakeDown from "./AdminTakeDown.vue";
import { useQueue } from "./useQueue";

/**
 * Notice and takedown: readers' reports of a cover. "Take down" removes the
 * picture everywhere for good (no refresh brings it back); "Keep it" closes
 * the report. Either decision closes every open report of the picture. Each
 * open report shows how long it has waited, flagged once that passes the
 * target. "Decided" lists the history, newest decision first, with the 90-day
 * figures. A notice that arrived by email enters through "Take down a picture
 * by URL".
 */
const props = defineProps<{
  /** Hours after which an open report is overdue (`AdminSummary.target_hours`). */
  targetHours: number;
  /** `AdminSummary.report_stats`, once loaded. */
  stats: AdminReportStats | null;
}>();
const emit = defineEmits<{ changed: [] }>();

const REASONS = ["wrong_book", "offensive", "copyright", "other"];

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const queue = useQueue<ReportOut>((view, page) => api.adminReportPage(view, page));
const { view, items, loaded, error, hasMore, loadingMore } = queue;
const busyId = ref<number | null>(null);
/** Taking a picture down needs a second tap. */
const confirming = ref<number | null>(null);
/** The time ages count to, set per load so a list does not shift while it is read. */
const now = ref(Date.now());

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  now.value = Date.now();
  await queue.load();
}

/** The report and every other open report of its picture, which the decision closed too. */
function dropPicture(report: ReportOut) {
  const picture = report.media_public_id;
  queue.drop((row) => row.id === report.id || (picture != null && row.media_public_id === picture));
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
    emit("changed");
  } catch (err) {
    // Resolved meanwhile (409): it leaves the open queue either way.
    if (err instanceof ApiError && err.status === 409) {
      queue.drop((row) => row.id === report.id);
      emit("changed");
    }
    toast.show(failed(err, t("admin.decideFailed")));
  } finally {
    busyId.value = null;
  }
}

function showView(next: "open" | "resolved") {
  confirming.value = null;
  now.value = Date.now();
  void queue.show(next);
}

/** A picture came down by URL: its open reports were closed too, and the counts changed. */
function tookDown() {
  emit("changed");
  void load();
}

function reason(code: string): string {
  return REASONS.includes(code) ? t(`cover.reasons.${code}`) : code;
}

function when(iso: string): string {
  return formatClubDate(iso, session.user?.timezone || "UTC", locale.locale);
}

function waited(report: ReportOut): string {
  return formatAge(hoursBetween(report.created_at, now.value));
}

function overdue(report: ReportOut): boolean {
  return isOverdue(report.created_at, props.targetHours, now.value);
}

/**
 * "Taken down by root · …" or "Kept by root · …", "someone" when that admin
 * has deleted the account; a report closed on arrival names nobody.
 */
function decision(report: ReportOut): string {
  if (report.closed_on_arrival) return t("admin.closedOnArrival");
  const name = report.resolved_by?.username ?? t("admin.someone");
  const at = report.resolved_at ? when(report.resolved_at) : "";
  return report.status === "removed" ? t("admin.takenDownBy", { name, when: at }) : t("admin.keptBy", { name, when: at });
}

/** "after 5 hours", for a report an admin decided. */
function took(report: ReportOut): string {
  if (report.closed_on_arrival || !report.resolved_at) return "";
  return t("admin.decidedAfter", { age: formatAge(hoursBetween(report.created_at, report.resolved_at)) });
}

function hours(value: number | null | undefined): string {
  return value == null ? "–" : new Intl.NumberFormat(locale.locale, { maximumFractionDigits: 1 }).format(value);
}

onMounted(load);
</script>

<template>
  <section class="admin-queue" aria-labelledby="admin-reports">
    <div class="section-head">
      <h2 id="admin-reports">{{ t("admin.reports") }}</h2>
    </div>
    <p class="fine muted">{{ t("admin.reportsBlurb") }}</p>

    <div class="segmented queue-views" role="group" :aria-label="t('admin.views')">
      <button type="button" :aria-pressed="view === 'open'" @click="showView('open')">{{ t("admin.openView") }}</button>
      <button type="button" :aria-pressed="view === 'resolved'" @click="showView('resolved')">{{ t("admin.history") }}</button>
    </div>

    <AdminTakeDown v-if="view === 'open'" @taken-down="tookDown" />

    <dl v-if="view === 'resolved' && stats" class="report-stats fine" :aria-label="t('admin.stats.title', { days: stats.days })">
      <div>
        <dt class="subtle">{{ t("admin.stats.resolved", { days: stats.days }) }}</dt>
        <dd class="nums">{{ stats.resolved }}</dd>
      </div>
      <div>
        <dt class="subtle">{{ t("admin.stats.median") }}</dt>
        <dd class="nums">{{ hours(stats.median_hours) }}</dd>
      </div>
      <div>
        <dt class="subtle">{{ t("admin.stats.max") }}</dt>
        <dd class="nums">{{ hours(stats.max_hours) }}</dd>
      </div>
      <div>
        <dt class="subtle">{{ t("admin.stats.overTarget", { hours: targetHours }) }}</dt>
        <dd class="nums" :class="{ late: stats.over_target > 0 }">{{ stats.over_target }}</dd>
      </div>
    </dl>

    <p v-if="error" class="error">
      {{ error }}
      <button class="text-btn" type="button" @click="load">{{ t("common.tryAgain") }}</button>
    </p>
    <div v-else-if="!loaded" class="skeleton admin-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0" class="fine subtle">
      {{ view === "open" ? t("admin.noReports") : t("admin.noHistory") }}
    </p>
    <ol v-else class="admin-list">
      <li
        v-for="report in items"
        :key="report.id"
        class="panel admin-item report-item"
        :class="{ overdue: view === 'open' && overdue(report) }"
      >
        <p class="admin-item-head fine">
          <span class="badge">{{ reason(report.reason) }}</span>
          <span v-if="view === 'resolved'" class="badge" :class="`status-${report.status}`">{{ t(`admin.status.${report.status}`) }}</span>
          <span class="subtle">
            {{ t("admin.byOn", { name: report.created_by?.username ?? t("admin.someone"), when: when(report.created_at) }) }}
          </span>
          <template v-if="view === 'open'">
            <span class="age subtle">{{ t("admin.waiting", { age: waited(report) }) }}</span>
            <span v-if="overdue(report)" class="badge overdue-badge">{{ t("admin.overdue") }}</span>
          </template>
        </p>
        <div class="report-body">
          <BookCover
            v-if="report.cover_url || report.cover_thumb_url"
            :book="{ title: report.book?.title ?? '', cover_url: report.cover_url, cover_thumb_url: report.cover_thumb_url }"
            size="md"
          />
          <div class="report-text">
            <RouterLink v-if="report.book" :to="bookPath(report.book)">{{ report.book.title }}</RouterLink>
            <p v-if="report.removed && view === 'open'" class="fine subtle">{{ t("admin.alreadyRemoved") }}</p>
            <p v-if="report.details" class="fine report-details">{{ report.details }}</p>
          </div>
        </div>
        <p v-if="view === 'resolved'" class="fine subtle decision">
          {{ decision(report) }}<template v-if="took(report)"> · {{ took(report) }}</template>
        </p>
        <template v-else>
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
        </template>
      </li>
    </ol>
    <button v-if="loaded && hasMore" class="btn btn-ghost btn-sm load-more" type="button" :disabled="loadingMore" @click="queue.more">
      {{ t("admin.loadMore") }}
    </button>
  </section>
</template>

<style scoped>
.queue-views {
  max-width: 22rem;
  margin: var(--space-3) 0;
}

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

.admin-item.overdue {
  border-color: var(--danger-border);
}

.admin-item-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}

.overdue-badge {
  color: var(--danger);
  background: transparent;
  box-shadow: inset 0 0 0 1px var(--danger-border);
}

.badge.status-removed {
  color: var(--danger);
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

.report-text p,
.decision {
  margin: 0;
}

.report-details {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.report-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(8.5rem, 1fr));
  gap: var(--space-2) var(--space-4);
  margin: 0 0 var(--space-3);
}

.report-stats div {
  display: grid;
  align-content: start;
  gap: 2px;
}

.report-stats dd {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: 600;
}

.report-stats dd.late {
  color: var(--danger);
}

.load-more {
  margin-top: var(--space-3);
}
</style>
