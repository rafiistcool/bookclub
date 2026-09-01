<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import { REACTIONS } from "../constants";
import { useToast } from "../stores/toast";
import type { Milestone, PickPost, PickThread } from "../types";
import Avatar from "./Avatar.vue";
import NavIcon from "./NavIcon.vue";

const props = defineProps<{
  pickId: number;
  /** Hide the composer even when the pick is open (past picks, other people's views). */
  readOnly?: boolean;
  milestones?: Milestone[];
  /** Restrict the thread to one milestone's notes. */
  milestoneId?: number | null;
  compact?: boolean;
}>();

const emit = defineEmits<{
  changed: [];
}>();

const thread = ref<PickThread | null>(null);
const error = ref("");
const draft = ref("");
const spoilerOn = ref(false);
const spoilerUpto = ref(50);
const composerMilestone = ref<number | null>(null);
const pending = ref(false);
const revealed = ref(new Set<number>());
const editing = ref<PickPost | null>(null);
const editDraft = ref("");
const pickerFor = ref<number | null>(null);
const toast = useToast();

const posts = computed(() => {
  const items = thread.value?.items ?? [];
  if (props.milestoneId == null) return items;
  return items.filter((post) => post.milestone_id === props.milestoneId);
});
const composerOpen = computed(() => Boolean(thread.value?.can_post) && !props.readOnly);
const myProgress = computed(() => thread.value?.my_progress ?? null);

function milestoneTitle(id: number | null): string {
  if (id == null) return "";
  return props.milestones?.find((row) => row.id === id)?.title ?? "";
}

/** A note is hidden when it is flagged beyond where the viewer is. */
function isSpoiler(post: PickPost): boolean {
  if (post.spoiler_upto == null || post.mine) return false;
  if (revealed.value.has(post.id)) return false;
  const mine = myProgress.value;
  if (mine == null) return true; // not on shelf / no progress yet: protect by default
  return post.spoiler_upto > mine;
}

async function load() {
  try {
    thread.value = await api.pickPosts(props.pickId);
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load notes";
  }
}

async function submit() {
  const body = draft.value.trim();
  if (!body || pending.value) return;
  pending.value = true;
  try {
    const created = await api.addPickPost(
      {
        body,
        spoiler_upto: spoilerOn.value ? spoilerUpto.value : null,
        milestone_id: props.milestoneId ?? composerMilestone.value,
      },
      props.pickId,
    );
    if (thread.value) thread.value.items = [...thread.value.items, created];
    draft.value = "";
    spoilerOn.value = false;
    emit("changed");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not post that");
  } finally {
    pending.value = false;
  }
}

function startEdit(post: PickPost) {
  editing.value = post;
  editDraft.value = post.body;
}

async function saveEdit() {
  const post = editing.value;
  if (!post || !editDraft.value.trim()) return;
  try {
    const updated = await api.editPickPost(post.id, { body: editDraft.value.trim() });
    replace(updated);
    editing.value = null;
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that");
  }
}

async function toggleSpoilerFlag(post: PickPost) {
  try {
    const updated = await api.editPickPost(post.id, {
      spoiler_upto: post.spoiler_upto == null ? Math.max(myProgress.value ?? 0, 10) : null,
    });
    replace(updated);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not update that");
  }
}

async function remove(post: PickPost) {
  const snapshot = post;
  if (!thread.value) return;
  thread.value.items = thread.value.items.filter((row) => row.id !== post.id);
  try {
    await api.deletePickPost(post.id);
    toast.show("Note deleted");
    emit("changed");
  } catch (err) {
    thread.value.items = [...thread.value.items, snapshot].sort((a, b) => a.id - b.id);
    toast.show(err instanceof ApiError ? err.message : "Could not delete that");
  }
}

async function react(post: PickPost, emoji: string) {
  pickerFor.value = null;
  try {
    replace(await api.toggleReaction(post.id, emoji));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not react");
  }
}

function replace(updated: PickPost) {
  if (!thread.value) return;
  thread.value.items = thread.value.items.map((row) => (row.id === updated.id ? updated : row));
}

onMounted(load);
watch(
  () => props.pickId,
  () => {
    thread.value = null;
    void load();
  },
);
defineExpose({ load });
</script>

<template>
  <section class="thread" :aria-label="milestoneId != null ? 'Milestone notes' : 'Notes'">
    <p v-if="error" class="error fine">{{ error }}</p>
    <div v-else-if="!thread" class="feed" aria-busy="true">
      <div v-for="n in 2" :key="n" class="note" style="border-color: transparent">
        <div class="skeleton circle" style="width: 32px; height: 32px" />
        <div style="display: grid; gap: 6px">
          <div class="skeleton line" style="width: 40%; height: 0.7em" />
          <div class="skeleton line" style="width: 90%" />
        </div>
      </div>
    </div>
    <template v-else>
      <p v-if="posts.length === 0 && !compact" class="muted fine">No notes yet. Start the thread.</p>
      <article v-for="post in posts" :key="post.id" class="note">
        <Avatar :username="post.author" size="sm" />
        <div style="min-width: 0">
          <div class="note-head">
            <span class="strong">{{ post.author }}</span>
            <span class="when">{{ post.created_label }}<template v-if="post.edited"> · edited</template></span>
            <span v-if="post.milestone_id != null && milestoneId == null && milestoneTitle(post.milestone_id)" class="badge">
              {{ milestoneTitle(post.milestone_id) }}
            </span>
            <span v-if="post.spoiler_upto != null" class="badge" :title="`Safe up to ${post.spoiler_upto}%`">
              ≤ {{ post.spoiler_upto }}%
            </span>
          </div>

          <template v-if="editing?.id === post.id">
            <textarea v-model="editDraft" class="input" rows="3" maxlength="1000" style="margin-top: 6px" />
            <div class="actions" style="margin-top: 8px">
              <button class="btn btn-primary btn-sm" type="button" @click="saveEdit">Save</button>
              <button class="btn btn-ghost btn-sm" type="button" @click="editing = null">Cancel</button>
            </div>
          </template>
          <template v-else>
            <p class="note-body" :class="{ 'hidden-spoiler': isSpoiler(post) }" :aria-hidden="isSpoiler(post)">
              {{ post.body }}
            </p>
            <div v-if="isSpoiler(post)" class="spoiler-cover">
              <span>
                Past {{ post.spoiler_upto }}% —
                <template v-if="myProgress == null">set your progress to see notes you’re ready for.</template>
                <template v-else>you’re at {{ myProgress }}%.</template>
              </span>
              <button class="text-btn sm" type="button" @click="revealed.add(post.id)">Show</button>
            </div>
          </template>

          <div class="note-foot">
            <button
              v-for="reaction in post.reactions"
              :key="reaction.emoji"
              class="reaction"
              :class="{ mine: reaction.mine }"
              type="button"
              :aria-pressed="reaction.mine"
              :title="reaction.users.join(', ')"
              @click="react(post, reaction.emoji)"
            >
              {{ reaction.emoji }} {{ reaction.count }}
            </button>
            <button
              class="reaction"
              type="button"
              aria-label="Add reaction"
              :aria-expanded="pickerFor === post.id"
              @click="pickerFor = pickerFor === post.id ? null : post.id"
            >
              <NavIcon name="plus" :size="14" />
            </button>
            <template v-if="post.mine">
              <button class="icon-btn" type="button" style="min-width: 36px; min-height: 32px" aria-label="Edit note" @click="startEdit(post)">
                <NavIcon name="edit" :size="16" />
              </button>
              <button
                class="text-btn sm"
                type="button"
                :title="post.spoiler_upto == null ? 'Flag as spoiler' : 'Remove spoiler flag'"
                @click="toggleSpoilerFlag(post)"
              >
                {{ post.spoiler_upto == null ? "Flag spoiler" : "Unflag" }}
              </button>
              <button class="icon-btn" type="button" style="min-width: 36px; min-height: 32px; color: var(--danger)" aria-label="Delete note" @click="remove(post)">
                <NavIcon name="trash" :size="16" />
              </button>
            </template>
          </div>
          <div v-if="pickerFor === post.id" class="reaction-picker" style="margin-top: 6px">
            <button v-for="emoji in REACTIONS" :key="emoji" type="button" :aria-label="`React ${emoji}`" @click="react(post, emoji)">
              {{ emoji }}
            </button>
          </div>
        </div>
      </article>

      <form v-if="composerOpen" class="composer" @submit.prevent="submit">
        <label class="field" style="margin: 0">
          <span class="visually-hidden">Add a note</span>
          <textarea
            v-model="draft"
            rows="2"
            maxlength="1000"
            :placeholder="milestoneId != null ? 'Thoughts on this stretch?' : 'A take, a quote, a question for the meeting…'"
          />
        </label>
        <div class="composer-row">
          <label class="chip" style="cursor: pointer">
            <input v-model="spoilerOn" type="checkbox" style="accent-color: var(--accent)" />
            Spoiler
          </label>
          <template v-if="spoilerOn">
            <span class="fine muted">safe up to</span>
            <input v-model.number="spoilerUpto" class="input" type="number" min="0" max="100" inputmode="numeric" aria-label="Safe up to percent" />
            <span class="fine muted">%</span>
          </template>
          <select
            v-if="milestoneId == null && milestones && milestones.length"
            v-model="composerMilestone"
            class="input"
            style="width: auto; min-height: 40px; padding: 6px 10px"
            aria-label="Attach to milestone"
          >
            <option :value="null">Whole book</option>
            <option v-for="row in milestones" :key="row.id" :value="row.id">{{ row.title }}</option>
          </select>
          <span style="flex: 1" />
          <button class="btn btn-primary" type="submit" :disabled="pending || !draft.trim()">Post</button>
        </div>
      </form>
      <p v-else-if="!compact && thread && !thread.can_post" class="faint fine">This pick is closed; notes are read-only.</p>
    </template>
  </section>
</template>
