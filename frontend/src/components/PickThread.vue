<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import type { PickPost } from "../types";

const props = defineProps<{
  pickId: number;
  canPost?: boolean;
}>();

const posts = ref<PickPost[]>([]);
const canPost = ref(false);
const draft = ref("");
const error = ref("");
const loaded = ref(false);
const pending = ref(false);
const toast = useToast();

async function load() {
  try {
    const thread = await api.pickPosts(props.pickId);
    posts.value = thread.items;
    canPost.value = thread.can_post && props.canPost !== false;
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
    const created = await api.addPickPost(body, props.pickId);
    posts.value = [...posts.value, created];
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
    void load();
  },
);
</script>

<template>
  <section class="pick-thread">
    <h2>Notes</h2>
    <p class="muted fine">
      A short take, quote, or meeting note. Oldest first.
    </p>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="empty">Loading…</div>
    <ol v-else-if="posts.length" class="pick-thread-list">
      <li v-for="post in posts" :key="post.id">
        <p class="pick-thread-meta">
          <strong>{{ post.author }}</strong>
          <span class="muted">{{ post.created_label }}</span>
        </p>
        <p class="pick-thread-body">{{ post.body }}</p>
      </li>
    </ol>
    <p v-else class="muted fine">No notes yet.</p>
    <form v-if="canPost" class="pick-thread-form" @submit.prevent="submit">
      <label class="field">
        <span>Add a note</span>
        <textarea
          v-model="draft"
          rows="3"
          maxlength="1000"
          placeholder="What stood out?"
        />
      </label>
      <button class="btn btn-primary" type="submit" :disabled="pending || !draft.trim()">
        Post
      </button>
    </form>
  </section>
</template>
