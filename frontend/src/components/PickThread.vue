<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import type { PickPost } from "../types";

const props = defineProps<{
  pickId: number;
  /** Show only the newest N posts until the reader asks for the rest. */
  preview?: number;
}>();

const posts = ref<PickPost[]>([]);
const open = ref(false);
const draft = ref("");
const error = ref("");
const loaded = ref(false);
const pending = ref(false);
const showAll = ref(false);
const toast = useToast();

const hidden = computed(() => {
  if (!props.preview || showAll.value) return 0;
  return Math.max(0, posts.value.length - props.preview);
});

const visible = computed(() =>
  hidden.value ? posts.value.slice(-props.preview!) : posts.value,
);

async function load() {
  try {
    const thread = await api.pickPosts(props.pickId);
    posts.value = thread.items;
    open.value = thread.can_post;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load notes";
  } finally {
    loaded.value = true;
  }
}

async function submit() {
  const body = draft.value.trim();
  if (!body || pending.value) return;
  pending.value = true;
  try {
    posts.value = [...posts.value, await api.addPickPost(body, props.pickId)];
    draft.value = "";
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not post that");
  } finally {
    pending.value = false;
  }
}

onMounted(load);
watch(
  () => props.pickId,
  () => {
    loaded.value = false;
    showAll.value = false;
    void load();
  },
);
</script>

<template>
  <section class="thread">
    <div class="section-head">
      <h2>Discussion</h2>
      <span v-if="posts.length" class="fine subtle nums">
        {{ posts.length }} {{ posts.length === 1 ? "note" : "notes" }}
      </span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton skeleton-block" aria-hidden="true" />

    <template v-else>
      <button
        v-if="hidden"
        class="text-btn show-all"
        type="button"
        @click="showAll = true"
      >
        Show {{ hidden }} earlier {{ hidden === 1 ? "note" : "notes" }}
      </button>
      <ol v-if="visible.length" class="thread-list">
        <li v-for="post in visible" :key="post.id">
          <p class="thread-meta">
            <strong>{{ post.author }}</strong>
            <span class="subtle finer">{{ post.created_label }}</span>
          </p>
          <p class="thread-body">{{ post.body }}</p>
        </li>
      </ol>
      <p v-else class="fine subtle">
        No notes yet. A quote, a reaction, or a question for the meeting.
      </p>
    </template>

    <form v-if="open" class="thread-form" @submit.prevent="submit">
      <label class="field">
        <span class="visually-hidden">Add a note</span>
        <textarea
          v-model="draft"
          rows="3"
          maxlength="1000"
          placeholder="A short take, a quote, or a meeting note"
        />
      </label>
      <button
        class="btn btn-primary btn-sm post-btn"
        type="submit"
        :disabled="pending || !draft.trim()"
      >
        Post
      </button>
    </form>
  </section>
</template>

<style scoped>
.thread {
  display: grid;
  gap: var(--space-3);
}

.skeleton-block {
  height: 90px;
}

.show-all {
  justify-self: start;
}

.thread-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.thread-list li {
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.thread-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-3);
  margin-bottom: var(--space-1);
}

.thread-body {
  white-space: pre-wrap;
  color: var(--text-muted);
}

.thread-form {
  display: grid;
  gap: var(--space-2);
}

.thread-form .field {
  margin-bottom: 0;
}

.post-btn {
  justify-self: start;
}

@media (min-width: 1024px) {
  .thread {
    max-width: 720px;
  }
}
</style>
