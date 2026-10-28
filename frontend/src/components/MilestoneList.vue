<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { formatClubDate } from "../i18n/dates";
import { tp } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useToast } from "../stores/toast";
import type { MilestoneOut, MilestoneUnit } from "../types";

/** Reading stops for the current pick: "up to chapter 5 by Friday". Admins add and remove them. */
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

const title = ref("");
const unit = ref<MilestoneUnit | "">("");
const rangeFrom = ref<number | "">("");
const rangeTo = ref<number | "">("");
const due = ref("");

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
  return formatClubDate(row.due_at, props.timezone, locale.locale) || row.due_label || "";
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
  busy.value = true;
  try {
    await api.deleteMilestone(props.slug, props.pickId, row.id);
    await load();
    toast.show(t("milestones.removed"));
  } catch (err) {
    toast.show(fail(err, t("milestones.saveFailed")));
  } finally {
    busy.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section class="milestones" aria-labelledby="milestones-heading">
    <div class="section-head">
      <h2 id="milestones-heading">{{ t("milestones.heading") }}</h2>
      <button v-if="canManage && !adding" class="text-btn" type="button" @click="adding = true">
        {{ t("milestones.add") }}
      </button>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton milestone-skeleton" aria-hidden="true" />
    <p v-else-if="items.length === 0 && !adding" class="fine subtle">
      {{ canManage ? t("milestones.emptyAdmin") : t("milestones.empty") }}
    </p>
    <ol v-else-if="items.length" class="milestone-list">
      <li v-for="row in items" :key="row.id" class="milestone" :class="{ passed: row.passed }">
        <span class="milestone-meta">
          <strong>{{ row.title }}</strong>
          <span class="finer subtle">
            <template v-if="range(row)">{{ range(row) }}</template>
            <template v-if="range(row) && dueLabel(row)"> · </template>
            <template v-if="dueLabel(row)">{{ t(row.passed ? "milestones.wasDue" : "milestones.due", { when: dueLabel(row) }) }}</template>
            <template v-if="row.note_count"> · {{ tp("milestones.notes", row.note_count) }}</template>
          </span>
        </span>
        <button
          v-if="canManage"
          class="text-btn"
          :class="{ danger: confirmingId === row.id }"
          type="button"
          :disabled="busy"
          @click="remove(row)"
        >
          {{ confirmingId === row.id ? t("common.deleteConfirm") : t("common.remove") }}
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

.milestone-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.milestone {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
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

.milestone-form {
  margin-top: var(--space-3);
}

.milestone-range {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-2);
}
</style>
