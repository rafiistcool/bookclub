<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import { STATUS_LABEL, fromNow, type Status } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import type { ActivityItem } from "../types";
import Avatar from "./Avatar.vue";
import BookCover from "./BookCover.vue";
import Skeleton from "./Skeleton.vue";
import StarRating from "./StarRating.vue";

const props = defineProps<{
  username?: string;
  limit?: number;
  /** Compact: no "load more", fewer rows (Home). */
  compact?: boolean;
}>();

const flow = useFlow();
const items = ref<ActivityItem[] | null>(null);
const hasMore = ref(false);
const error = ref("");
const loadingMore = ref(false);

const pageSize = computed(() => props.limit ?? (props.compact ? 6 : 30));

async function load() {
  try {
    const page = await api.activity({ limit: pageSize.value, username: props.username });
    items.value = page.items;
    hasMore.value = page.has_more;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load activity";
  }
}

async function more() {
  if (!items.value?.length || loadingMore.value) return;
  loadingMore.value = true;
  try {
    const last = items.value[items.value.length - 1].id;
    const page = await api.activity({ limit: pageSize.value, before: last, username: props.username });
    items.value = [...items.value, ...page.items];
    hasMore.value = page.has_more;
  } finally {
    loadingMore.value = false;
  }
}

function verb(item: ActivityItem): string {
  const p = item.payload as Record<string, string | number | boolean | undefined>;
  const status = p.status as Status | undefined;
  switch (item.kind) {
    case "member_joined":
      return "joined the club";
    case "shelf_added":
      return status ? `added to ${STATUS_LABEL[status]}` : "added a book";
    case "shelf_moved":
      return status ? `moved to ${STATUS_LABEL[status]}` : "moved a book";
    case "shelf_finished":
      return "finished";
    case "shelf_dnf":
      return "did not finish";
    case "shelf_removed":
      return "removed";
    case "progress":
      return `is ${p.progress}% through`;
    case "pick_set":
      return "set the club pick";
    case "pick_cleared":
      return "cleared the club pick";
    case "note_posted":
      return "posted a note on";
    case "reaction":
      return `reacted ${p.emoji ?? ""} to ${p.author ?? "a"}${p.author ? "’s" : ""} note on`;
    case "nominated":
      return "nominated";
    case "voted":
      return "voted for";
    case "vote_closed":
      return p.auto ? "won the vote (deadline)" : "closed the vote for";
    case "milestone_added":
      return `added a milestone${p.title ? ` “${p.title}”` : ""} to`;
    case "quote_added":
      return "saved a quote from";
    default:
      return item.kind.replace(/_/g, " ");
  }
}

onMounted(load);
watch(() => props.username, load);
defineExpose({ load });
</script>

<template>
  <div>
    <p v-if="error" class="error fine">{{ error }}</p>
    <Skeleton v-else-if="!items" kind="feed" :count="compact ? 4 : 8" />
    <p v-else-if="items.length === 0" class="muted fine">Nothing has happened yet. Add a book to get things going.</p>
    <div v-else class="feed">
      <article v-for="item in items" :key="item.id" class="feed-item">
        <Avatar :username="item.actor" size="sm" />
        <div class="feed-text">
          <span class="who">{{ item.mine ? "You" : item.actor }}</span>
          {{ " " }}{{ verb(item) }}
          <template v-if="item.book">
            {{ " " }}
            <button
              type="button"
              class="strong"
              style="all: unset; cursor: pointer; font-weight: 600"
              @click="flow.open({ kind: 'details', book: toRef(item.book) })"
            >
              {{ item.book.title }}
            </button>
          </template>
          <template v-if="item.kind === 'shelf_finished' && item.payload.rating">
            {{ " " }}<StarRating :value="Number(item.payload.rating)" />
          </template>
          <span v-if="item.kind === 'shelf_finished' && item.payload.take" class="quote"> — “{{ item.payload.take }}”</span>
          <span v-if="item.kind === 'quote_added' && item.payload.excerpt" class="quote"> “{{ item.payload.excerpt }}”</span>
          <span v-if="item.kind === 'shelf_dnf' && item.payload.reason" class="quote"> — {{ item.payload.reason }}</span>
          <span class="when" :title="item.created_label">{{ fromNow(item.created_at) }}</span>
        </div>
        <button
          v-if="item.book"
          type="button"
          style="all: unset; cursor: pointer"
          :aria-label="`Details for ${item.book.title}`"
          @click="flow.open({ kind: 'details', book: toRef(item.book) })"
        >
          <BookCover :title="item.book.title" :cover-id="item.book.cover_id" size="xs" />
        </button>
      </article>
    </div>
    <div v-if="items && hasMore && !compact" class="library-more">
      <button class="btn btn-ghost btn-sm" type="button" :disabled="loadingMore" @click="more">
        {{ loadingMore ? "Loading…" : "Show older" }}
      </button>
    </div>
  </div>
</template>
