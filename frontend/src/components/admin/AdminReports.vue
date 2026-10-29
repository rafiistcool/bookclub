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
import Avatar from "../Avatar.vue";
import BookCover from "../BookCover.vue";
import AdminTakeDown from "./AdminTakeDown.vue";
import { useQueue } from "./useQueue";

/**
 * Notice and takedown: readers' reports of a cover, a profile picture, or a
 * book or edition a reader added (`ReportOut.target_kind`). For a picture,
 * "Take down" removes it everywhere for good (no refresh brings it back); for
 * a book or edition, "Remove the text" withdraws what the reader typed and
 * keeps the record. "Keep it" closes the report. Either decision closes every
 * open report of the same picture, book or edition, and the server mails the
 * reporters and, on a removal, whoever's content it was. Each open report
 * shows how long it has waited, flagged once that passes the target.
 * "Decided" lists the history, newest decision first, with the 90-day
 * figures. A notice that arrived by email enters through "Take down a picture
 * by URL" (pictures; it names no sender) or as a report an admin files.
 *
 * A report of something gone (`ReportOut.gone`: a profile picture its owner
 * replaced or deleted, a book merged into another one since) says so and offers
 * only "Close the report": nothing is left to take down, and the server refuses
 * "remove" for it.
 */
const props = defineProps<{
  /** Hours after which an open report is overdue (`AdminSummary.target_hours`). */
  targetHours: number;
  /** `AdminSummary.report_stats`, once loaded. */
  stats: AdminReportStats | null;
}>();
const emit = defineEmits<{ changed: [] }>();

const REASONS = ["wrong_book", "offensive", "copyright", "other"];
const KINDS = ["cover", "avatar", "work", "edition"];

/** A picture is taken down; a book or edition loses its text. */
function isPicture(report: ReportOut): boolean {
  return report.target_kind !== "work" && report.target_kind !== "edition";
}

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

/**
 * The report and every other open report of its picture, which the decision
 * closed too. A book's or edition's other reports, and those of a gone
 * profile picture (which have no picture id), are not told apart by the list
 * (an edition report carries its book), so the queue is read again.
 */
async function dropDecided(report: ReportOut) {
  if (!isPicture(report) || report.gone) {
    queue.drop((row) => row.id === report.id);
    await load();
    return;
  }
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
    if (action === "remove") toast.show(isPicture(report) ? t("admin.takenDown") : t("admin.textRemoved"));
    else toast.show(isPicture(report) && !report.gone ? t("admin.kept") : t("admin.reportClosed"));
    emit("changed");
    await dropDecided(report);
  } catch (err) {
    // 409: resolved meanwhile, or gone since the list was read (the picture
    // replaced, the book merged). The queue read again shows which.
    if (err instanceof ApiError && err.status === 409) {
      emit("changed");
      await load();
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

function kind(report: ReportOut): string {
  return KINDS.includes(report.target_kind) ? t(`admin.target.${report.target_kind}`) : report.target_kind;
}

/** Who reported it, and when; an emailed notice names no account. */
function source(report: ReportOut): string {
  if (report.via === "notice") return t("admin.notice", { when: when(report.created_at) });
  return t("admin.byOn", { name: report.created_by?.username ?? t("admin.someone"), when: when(report.created_at) });
}

/** What "remove" does, shown before the second tap. */
function removeBlurb(report: ReportOut): string {
  return report.target_kind === "cover" || !KINDS.includes(report.target_kind)
    ? t("admin.takeDownBlurb")
    : t(`admin.removeBlurb.${report.target_kind}`);
}

/** Why a gone report has nothing left to decide. */
function goneNote(report: ReportOut): string {
  return t(report.target_kind === "avatar" ? "admin.gone.avatar" : "admin.gone.work");
}

function statusLabel(report: ReportOut): string {
  if (report.status === "removed" && !isPicture(report)) return t("admin.status.withdrawn");
  return t(`admin.status.${report.status}`);
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
 * "Taken down by root · …" (a picture), "Removed by root · …" (a book's
 * text) or "Kept by root · …", "someone" when that admin has deleted the
 * account; a report closed on arrival names nobody.
 */
function decision(report: ReportOut): string {
  if (report.closed_on_arrival) return t("admin.closedOnArrival");
  const name = report.resolved_by?.username ?? t("admin.someone");
  const at = report.resolved_at ? when(report.resolved_at) : "";
  if (report.status !== "removed") return t(report.gone ? "admin.closedBy" : "admin.keptBy", { name, when: at });
  return isPicture(report) ? t("admin.takenDownBy", { name, when: at }) : t("admin.removedBy", { name, when: at });
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
      <h2 id="admin-reports">{{ t("admin.tabs.reports") }}</h2>
    </div>
    <p class="fine muted">{{ t("admin.reportsIntro") }}</p>

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
          <span class="badge kind-badge">{{ kind(report) }}</span>
          <span class="badge">{{ reason(report.reason) }}</span>
          <span v-if="view === 'resolved'" class="badge" :class="`status-${report.status}`">{{ statusLabel(report) }}</span>
          <span class="subtle">{{ source(report) }}</span>
          <template v-if="view === 'open'">
            <span class="age subtle">{{ t("admin.waiting", { age: waited(report) }) }}</span>
            <span v-if="overdue(report)" class="badge overdue-badge">{{ t("admin.overdue") }}</span>
          </template>
        </p>
        <div class="report-body">
          <Avatar
            v-if="report.target_kind === 'avatar'"
            :username="report.target_user?.username ?? '?'"
            :src="report.avatar_url"
            size="lg"
          />
          <BookCover
            v-else-if="report.cover_url || report.cover_thumb_url"
            :book="{ title: report.book?.title ?? '', cover_url: report.cover_url, cover_thumb_url: report.cover_thumb_url }"
            size="md"
          />
          <div class="report-text">
            <p v-if="report.target_kind === 'avatar'" class="report-subject">
              {{ t("admin.avatarOf", { name: report.target_user?.username ?? t("admin.someone") }) }}
            </p>
            <RouterLink v-else-if="report.book" :to="bookPath(report.book)">{{ report.book.title }}</RouterLink>
            <p v-if="report.removed && view === 'open'" class="fine subtle">{{ t("admin.alreadyRemoved") }}</p>
            <p v-if="report.gone && view === 'open'" class="fine subtle gone-note">{{ goneNote(report) }}</p>
            <p v-if="report.details" class="fine report-details">{{ report.details }}</p>
          </div>
        </div>
        <p v-if="view === 'resolved'" class="fine subtle decision">
          {{ decision(report) }}<template v-if="took(report)"> · {{ took(report) }}</template>
        </p>
        <div v-else-if="report.gone" class="btn-row">
          <button class="btn btn-ghost btn-sm" type="button" :disabled="busyId === report.id" @click="resolve(report, 'dismiss')">
            {{ t("admin.closeReport") }}
          </button>
        </div>
        <template v-else>
          <p v-if="confirming === report.id" class="fine">{{ removeBlurb(report) }}</p>
          <div class="btn-row">
            <button class="btn btn-danger btn-sm" type="button" :disabled="busyId === report.id" @click="resolve(report, 'remove')">
              <template v-if="isPicture(report)">
                {{ confirming === report.id ? t("admin.takeDownConfirm") : t("admin.takeDown") }}
              </template>
              <template v-else>
                {{ confirming === report.id ? t("admin.removeTextConfirm") : t("admin.removeText") }}
              </template>
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

.kind-badge {
  background: transparent;
  box-shadow: inset 0 0 0 1px var(--border);
}

.report-subject {
  font-weight: 600;
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
