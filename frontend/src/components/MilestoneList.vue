<script setup lang="ts">
import { ref } from "vue";
import { api, ApiError } from "../api/client";
import { countdown } from "../constants";
import { useToast } from "../stores/toast";
import type { Milestone } from "../types";
import NavIcon from "./NavIcon.vue";
import PickThread from "./PickThread.vue";
import Sheet from "./Sheet.vue";

const props = defineProps<{
  pickId: number;
  milestones: Milestone[];
  timezone: string;
  canEdit: boolean;
}>();

const emit = defineEmits<{
  changed: [];
}>();

const open = ref<number | null>(null);
const adding = ref(false);
const editingRow = ref<Milestone | null>(null);
const title = ref("");
const from = ref<number | "">("");
const to = ref<number | "">("");
const due = ref("");
const pending = ref(false);
const toast = useToast();

function startAdd() {
  editingRow.value = null;
  title.value = `Part ${props.milestones.length + 1}`;
  from.value = "";
  to.value = "";
  due.value = "";
  adding.value = true;
}

function startEdit(row: Milestone) {
  editingRow.value = row;
  title.value = row.title;
  from.value = row.chapter_from ?? "";
  to.value = row.chapter_to ?? "";
  due.value = row.due_local ?? "";
  adding.value = true;
}

async function save() {
  if (!title.value.trim() || pending.value) return;
  pending.value = true;
  const body = {
    title: title.value.trim(),
    chapter_from: from.value === "" ? null : Number(from.value),
    chapter_to: to.value === "" ? null : Number(to.value),
    due_at: due.value || null,
  };
  try {
    if (editingRow.value) await api.editMilestone(props.pickId, editingRow.value.id, body);
    else await api.addMilestone(props.pickId, body);
    adding.value = false;
    emit("changed");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save the milestone");
  } finally {
    pending.value = false;
  }
}

async function remove(row: Milestone) {
  try {
    await api.deleteMilestone(props.pickId, row.id);
    if (open.value === row.id) open.value = null;
    emit("changed");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not delete the milestone");
  }
}

function range(row: Milestone): string {
  if (row.chapter_from != null && row.chapter_to != null) return `Ch. ${row.chapter_from}–${row.chapter_to}`;
  if (row.chapter_from != null) return `From ch. ${row.chapter_from}`;
  if (row.chapter_to != null) return `To ch. ${row.chapter_to}`;
  return "";
}
</script>

<template>
  <section class="section">
    <div class="section-title">
      <h2>Reading schedule</h2>
      <button v-if="canEdit" class="text-btn sm" type="button" aria-label="Add milestone" @click="startAdd">
        <NavIcon name="plus" :size="16" /> Add
      </button>
    </div>
    <p v-if="milestones.length === 0" class="muted fine">
      Split the book into stretches with dates. Each one gets its own notes — spoiler-safe by design.
    </p>
    <ol v-else class="list" style="list-style: none; padding: 0">
      <li v-for="row in milestones" :key="row.id" class="card" style="padding: 10px 12px">
        <button
          type="button"
          style="all: unset; cursor: pointer; display: grid; grid-template-columns: 1fr auto; gap: 10px; width: 100%; align-items: center"
          :aria-expanded="open === row.id"
          @click="open = open === row.id ? null : row.id"
        >
          <span style="min-width: 0">
            <span class="strong">{{ row.title }}</span>
            <span v-if="range(row)" class="muted fine"> · {{ range(row) }}</span>
            <span class="fine" style="display: block" :class="row.passed ? 'faint' : 'muted'">
              <template v-if="row.due_label">
                {{ row.due_label }}
                <template v-if="countdown(row.due_at)"> · {{ countdown(row.due_at)!.label }}</template>
              </template>
              <template v-else>No date</template>
              <template v-if="row.note_count"> · {{ row.note_count }} {{ row.note_count === 1 ? "note" : "notes" }}</template>
            </span>
          </span>
          <NavIcon :name="open === row.id ? 'close' : 'quote'" :size="18" />
        </button>
        <div v-if="open === row.id" style="margin-top: 10px">
          <div v-if="canEdit" class="actions" style="margin-bottom: 8px">
            <button class="btn btn-ghost btn-sm" type="button" @click="startEdit(row)">Edit</button>
            <button class="btn btn-danger btn-sm" type="button" @click="remove(row)">Delete</button>
          </div>
          <PickThread :pick-id="pickId" :milestone-id="row.id" :read-only="!canEdit" compact @changed="emit('changed')" />
        </div>
      </li>
    </ol>

    <Sheet v-if="adding" :title="editingRow ? 'Edit milestone' : 'Add milestone'" @close="adding = false">
      <label class="field">
        <span>Title</span>
        <input v-model="title" maxlength="120" data-autofocus />
      </label>
      <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10px">
        <label class="field">
          <span>From chapter</span>
          <input v-model="from" type="number" inputmode="numeric" min="0" />
        </label>
        <label class="field">
          <span>To chapter</span>
          <input v-model="to" type="number" inputmode="numeric" min="0" />
        </label>
      </div>
      <label class="field">
        <span>Read by <span class="faint">({{ timezone }})</span></span>
        <input v-model="due" type="datetime-local" />
      </label>
      <template #foot>
        <button class="btn btn-primary" type="button" :disabled="pending || !title.trim()" @click="save">Save</button>
      </template>
    </Sheet>
  </section>
</template>
