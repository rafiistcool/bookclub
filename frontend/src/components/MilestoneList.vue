<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { focusLost, focusTemporarily } from "../focusBack";
import { formatEventTime } from "../i18n/dates";
import { tp } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useToast } from "../stores/toast";
import type { MilestoneOut, MilestoneUnit } from "../types";

/**
 * Reading stops for the current pick: "up to chapter 5 by Friday". Admins add and remove them.
 * A member, who can't add any, sees nothing at all (no heading) while there are none.
 * Removing is a quiet text action that shows only after "Edit" in the heading, so three
 * stops never come with three "Remove"s; it asks once more before it removes.
 */
const props = defineProps<{
  slug: string;
  pickId: number;
  canManage: boolean;
  timezone: string;
}>();

const { t } = useI18n();
const locale = useLocale();
const clubs = useClubs();
const toast = useToast();

const items = ref<MilestoneOut[]>([]);
const loaded = ref(false);
const error = ref("");
const adding = ref(false);
const busy = ref(false);
const confirmingId = ref<number | null>(null);
/** "Edit" in the heading shows each stop's "Remove". */
const editing = ref(false);
const root = ref<HTMLElement | null>(null);

const title = ref("");
const unit = ref<MilestoneUnit | "">("");
const rangeFrom = ref<number | "">("");
const rangeTo = ref<number | "">("");
const due = ref("");

const shown = computed(() => props.canManage || items.value.length > 0 || error.value !== "");

function fail(err: unknown, fallback: string): string {
  if (isClubGone(err)) clubs.markGone(props.slug);
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    items.value = (await api.milestones(props.slug, props.pickId)).items;
    error.value = "";
  } catch (err) {
    error.value = fail(err, t("milestones.loadFailed"));
  } finally {
    loaded.value = true;
  }
}

function range(row: MilestoneOut): string {
  if (!row.unit || (row.range_from == null && row.range_to == null)) return "";
  const key = row.unit === "percent" ? "milestones.percentRange" : "milestones.chapterRange";
  const from = row.range_from ?? row.range_to;
  const to = row.range_to ?? row.range_from;
  return from === to ? t(`${key}One`, { n: to }) : t(key, { from, to });
}

function dueLabel(row: MilestoneOut): string {
  if (!row.due_at) return "";
  return formatEventTime(row.due_at, props.timezone, locale.locale) || row.due_label || "";
}

/** The meta line's facts. Each wraps as a unit: a time keeps its zone, a count its noun. */
function metaParts(row: MilestoneOut): string[] {
  const when = dueLabel(row);
  return [
    range(row),
    when ? t(row.passed ? "milestones.wasDue" : "milestones.due", { when }) : "",
    row.note_count ? tp("milestones.notes", row.note_count) : "",
  ].filter(Boolean);
}

function reset() {
  title.value = "";
  unit.value = "";
  rangeFrom.value = "";
  rangeTo.value = "";
  due.value = "";
}

async function add() {
  const name = title.value.trim();
  if (!name || busy.value) return;
  busy.value = true;
  try {
    const created = await api.addMilestone(props.slug, props.pickId, {
      title: name,
      unit: unit.value || null,
      range_from: unit.value && rangeFrom.value !== "" ? Number(rangeFrom.value) : null,
      range_to: unit.value && rangeTo.value !== "" ? Number(rangeTo.value) : null,
      due_at: due.value.trim() || null,
    });
    items.value = [...items.value, created].sort((a, b) => a.position - b.position);
    reset();
    adding.value = false;
    toast.show(t("milestones.added"));
  } catch (err) {
    toast.show(fail(err, t("milestones.saveFailed")));
  } finally {
    busy.value = false;
  }
}

async function remove(row: MilestoneOut) {
  if (confirmingId.value !== row.id) {
    confirmingId.value = row.id;
    return;
  }
  confirmingId.value = null;
  // The tapped "Remove" goes with its row: focus moves on to the next stop's
  // (else the one before), and with the last stop gone to "Add".
  const at = items.value.findIndex((item) => item.id === row.id);
  const neighbour = items.value[at + 1] ?? items.value[at - 1] ?? null;
  busy.value = true;
  try {
    await api.deleteMilestone(props.slug, props.pickId, row.id);
    await load();
    if (items.value.length === 0) editing.value = false;
    busy.value = false;
    await nextTick();
    if (focusLost()) focusAfterRemoval(neighbour?.id ?? null);
    toast.show(t("milestones.removed"));
  } catch (err) {
    toast.show(fail(err, t("milestones.saveFailed")));
  } finally {
    busy.value = false;
  }
}

/** Where focus goes after a stop was removed: its neighbour's "Remove", else "Add", else the heading. */
function focusAfterRemoval(neighbourId: number | null) {
  const section = root.value;
  if (!section) return;
  const next =
    (neighbourId != null && section.querySelector<HTMLElement>(`[data-milestone="${neighbourId}"] .milestone-remove`)) ||
    section.querySelector<HTMLElement>(".milestone-add");
  if (next) next.focus();
  else {
    const heading = section.querySelector<HTMLElement>("#milestones-heading");
    if (heading) focusTemporarily(heading);
  }
}

onMounted(load);
</script>

<template>
  <section v-if="shown" ref="root" class="milestones" aria-labelledby="milestones-heading">
    <div class="section-head">
      <h2 id="milestones-heading">{{ t("milestones.heading") }}</h2>
      <span v-if="canManage" class="milestone-tools">
        <button
          v-if="items.length && !adding"
          class="text-btn"
          type="button"
          @click="
            editing = !editing;
            confirmingId = null;
          "
        >
          {{ editing ? t("milestones.doneEditing") : t("common.edit") }}
        </button>
        <button v-if="!adding && !editing" class="text-btn milestone-add" type="button" @click="adding = true">
          {{ t("milestones.add") }}
        </button>
      </span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton milestone-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0 && !adding" class="fine subtle">
      {{ t("milestones.emptyAdmin") }}
    </p>
    <ol v-else-if="items.length" class="milestone-list">
      <li v-for="row in items" :key="row.id" class="milestone" :class="{ passed: row.passed }" :data-milestone="row.id">
        <span class="milestone-meta">
          <strong :id="`milestone-${row.id}`">{{ row.title }}</strong>
          <span v-if="metaParts(row).length" class="milestone-facts finer subtle nums">
            <span v-for="part in metaParts(row)" :key="part">{{ part }}</span>
          </span>
        </span>
        <button
          v-if="canManage && editing"
          class="text-btn milestone-remove"
          :class="{ danger: confirmingId === row.id }"
          type="button"
          :aria-describedby="`milestone-${row.id}`"
          :disabled="busy"
          @click="remove(row)"
        >
          {{ confirmingId === row.id ? t("milestones.removeConfirm") : t("common.remove") }}
        </button>
      </li>
    </ol>

    <form v-if="adding" class="milestone-form panel" @submit.prevent="add">
      <label class="field">
        <span>{{ t("milestones.title") }}</span>
        <input v-model="title" maxlength="120" required :placeholder="t('milestones.titlePlaceholder')" />
      </label>
      <div class="milestone-range">
        <label class="field">
          <span>{{ t("milestones.unit") }}</span>
          <select v-model="unit">
            <option value="">{{ t("milestones.noUnit") }}</option>
            <option value="chapter">{{ t("milestones.chapters") }}</option>
            <option value="percent">{{ t("milestones.percent") }}</option>
          </select>
        </label>
        <label v-if="unit" class="field">
          <span>{{ t("milestones.from") }}</span>
          <input v-model.number="rangeFrom" type="number" min="0" :max="unit === 'percent' ? 100 : undefined" inputmode="numeric" />
        </label>
        <label v-if="unit" class="field">
          <span>{{ t("milestones.to") }}</span>
          <input v-model.number="rangeTo" type="number" min="0" :max="unit === 'percent' ? 100 : undefined" inputmode="numeric" />
        </label>
      </div>
      <label class="field">
        <span>{{ t("milestones.dueLabel") }}</span>
        <input v-model="due" type="datetime-local" />
        <span class="field-hint">{{ t("meeting.optionalTz", { tz: timezone }) }}</span>
      </label>
      <div class="btn-row">
        <button class="btn btn-primary btn-sm" type="submit" :disabled="busy || !title.trim()">
          {{ t("milestones.save") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="adding = false; reset()">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>
  </section>
</template>

<style scoped>
.milestone-skeleton {
  height: 70px;
}

/* Light rows split by hairlines, like the rest of the club home. */
.milestone-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.milestone {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) 0;
}

.milestone + .milestone {
  border-top: 1px solid var(--border);
}

.milestone-tools {
  display: inline-flex;
  gap: var(--space-4);
}

/* Only in edit mode, and quiet there: regular weight, muted, until it asks
   to be confirmed. The padding only widens its target; the text lines up
   with the row's edge. */
.milestone-remove {
  flex: none;
  margin-inline-end: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
  font-weight: 400;
  color: var(--text-muted);
}

.milestone-remove.danger {
  font-weight: 600;
  color: var(--danger);
}

.milestone.passed {
  opacity: 0.7;
}

.milestone-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.milestone-meta strong {
  overflow-wrap: anywhere;
}

/* Facts joined by dots. As flex items they wrap as units (a time keeps its
   zone, a count its noun), and a dot stays at the end of its line instead of
   starting the next one. */
.milestone-facts {
  display: flex;
  flex-wrap: wrap;
  column-gap: 0.3em;
}

.milestone-facts > span:not(:last-child)::after {
  content: " ·";
}

.milestone-form {
  margin-top: var(--space-3);
}

.milestone-range {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-2);
}
</style>
