<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { clubFeedPreview, EXCERPT_LENGTH, isShielded, positionMarker } from "../diary";
import { formatClubDate } from "../i18n/dates";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { postTimeZone } from "../timezone";
import type { DiaryFeedItem } from "../types";
import BookCover from "./BookCover.vue";

/**
 * "Recently written": diary entries and club posts, newest first. Without
 * `club` it is the combined feed of Home (every club of the viewer); with a
 * slug, that club's posts and its members' diary entries. With `mine`, only
 * the viewer's own diary writing (`/diary`), each root with its "Only me" switch.
 */
const props = defineProps<{
  club?: string;
  mine?: boolean;
  heading?: string;
  /** Shown when nothing is written yet. */
  emptyText?: string;
  pageSize?: number;
}>();

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const clubs = useClubs();
const toast = useToast();

const feed = ref<DiaryFeedItem[]>([]);
/** The last entry id read. */
const cursor = ref<number | undefined>();
const error = ref("");
const loaded = ref(false);
const hasMore = ref(false);
const busy = ref(false);

async function load(more = false) {
  if (busy.value) return;
  busy.value = true;
  try {
    const before = more ? cursor.value : undefined;
    const page = props.mine
      ? await api.diaryFeed(before, props.pageSize ?? 10, undefined, true)
      : await api.diaryFeed(before, props.pageSize ?? 8, props.club);
    feed.value = more ? [...feed.value, ...page.items] : page.items;
    hasMore.value = page.has_more;
    cursor.value = page.items[page.items.length - 1]?.entry.id ?? before;
    error.value = "";
  } catch (err) {
    if (props.club && isClubGone(err)) clubs.markGone(props.club);
    error.value = err instanceof ApiError ? err.message : t("club.diaryLoadFailed");
  } finally {
    loaded.value = true;
    busy.value = false;
  }
}

const expanded = ref<Set<number>>(new Set());
// Notes the two-line preview cannot show in full: cut by length, or clamped at this width.
const clamped = ref<Set<number>>(new Set());

function measureBody(id: number, el: HTMLElement | null) {
  if (!el || expanded.value.has(id)) return;
  const isClamped = el.scrollHeight > el.clientHeight + 1;
  if (isClamped !== clamped.value.has(id)) {
    const next = new Set(clamped.value);
    if (isClamped) next.add(id);
    else next.delete(id);
    clamped.value = next;
  }
}

function expandable(item: DiaryFeedItem): boolean {
  if (item.entry.deleted || isShielded(item.entry, item.my_progress, item.my_status)) return false;
  if (expanded.value.has(item.entry.id)) return true;
  const body = item.entry.body.trim();
  return body.length > EXCERPT_LENGTH || body.includes("\n") || clamped.value.has(item.entry.id);
}

function toggle(id: number) {
  const next = new Set(expanded.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  expanded.value = next;
}

function entryPath(item: DiaryFeedItem): string {
  return `${bookPath(item.book)}#entry-${item.entry.id}`;
}

const switching = ref<Set<number>>(new Set());

/** `mine`: a root's "Only me" switch (replies have no visibility of their own). */
async function setOnlyMe(item: DiaryFeedItem, onlyMe: boolean) {
  const id = item.entry.id;
  if (switching.value.has(id)) return;
  switching.value = new Set([...switching.value, id]);
  try {
    const saved = await api.editDiaryEntry(id, { visibility: onlyMe ? "only_me" : "audience" });
    feed.value = feed.value.map((row) =>
      row.entry.id === id ? { ...row, entry: { ...row.entry, visibility: saved.visibility } } : row,
    );
    toast.show(t("diary.updated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("diary.saveFailed"));
  } finally {
    const next = new Set(switching.value);
    next.delete(id);
    switching.value = next;
  }
}

onMounted(() => void load());
watch(
  () => props.club,
  () => {
    loaded.value = false;
    feed.value = [];
    cursor.value = undefined;
    void load();
  },
);
defineExpose({ reload: () => load() });
</script>

<template>
  <section class="diary-feed">
    <div class="section-head">
      <h2>{{ heading ?? t("club.recentlyWritten") }}</h2>
      <slot name="head" />
    </div>
    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton feed-skeleton" aria-hidden="true" />
    <div v-else-if="feed.length === 0" class="feed-empty">
      <p class="fine subtle">{{ emptyText ?? t("club.nothingWritten") }}</p>
      <slot name="empty" />
    </div>
    <template v-else>
      <ol class="feed-list">
        <li v-for="item in feed" :key="item.entry.id">
          <!-- Not one big link: the note can be unfolded in place. -->
          <article class="feed-row">
            <RouterLink class="feed-cover" :to="entryPath(item)" tabindex="-1" aria-hidden="true">
              <BookCover :book="item.book" size="xs" />
            </RouterLink>
            <span class="feed-meta">
              <RouterLink v-if="mine" class="feed-head" :to="entryPath(item)">
                <strong>{{ item.book.title }}</strong>
                <span v-if="item.entry.parent_id != null" class="finer subtle">
                  {{ item.parent_author ? t("diaryPage.replyTo", { name: item.parent_author }) : t("diaryPage.reply") }}
                </span>
              </RouterLink>
              <RouterLink v-else class="feed-head" :to="entryPath(item)">
                <strong>{{ item.entry.author ?? t("diary.deletedAccount") }}</strong>
                <span class="finer subtle">
                  <template v-if="item.parent_author">
                    {{ t("club.repliedTo", { name: item.parent_author }) }}
                  </template>
                  <template v-else>{{ t("club.on") }}</template>
                  {{ item.book.title }}
                </span>
              </RouterLink>
              <span
                :ref="(el) => measureBody(item.entry.id, el as HTMLElement | null)"
                class="feed-body"
                :class="{
                  'clamp-2': !expanded.has(item.entry.id),
                  expanded: expanded.has(item.entry.id),
                  shielded: isShielded(item.entry, item.my_progress, item.my_status),
                }"
              >
                <span
                  v-if="item.entry.visibility === 'only_me'"
                  class="badge only-me-badge"
                  :title="t('diary.onlyMeHint')"
                >
                  {{ t("diary.onlyMeBadge") }}
                </span>
                <span
                  v-if="item.entry.spoiler_upto != null"
                  class="badge"
                  :title="t('club.safeUptoTitle', { n: item.entry.spoiler_upto })"
                >
                  {{ t("club.spoilerFlagged") }}
                </span>
                {{ expanded.has(item.entry.id) ? item.entry.body.trim() : clubFeedPreview(item) }}
              </span>
              <button
                v-if="expandable(item)"
                class="text-btn feed-more"
                type="button"
                :aria-expanded="expanded.has(item.entry.id)"
                @click="toggle(item.entry.id)"
              >
                {{ expanded.has(item.entry.id) ? t("common.showLess") : t("common.showMore") }}
              </button>
              <span class="finer subtle feed-foot">
                <template v-if="!club && item.club">
                  <RouterLink class="feed-club" :to="`/c/${item.club.slug}`">{{ item.club.name }}</RouterLink> ·
                </template>
                <template v-if="positionMarker(item.entry)">
                  {{ positionMarker(item.entry) }} ·
                </template>
                {{ formatClubDate(item.entry.created_at, postTimeZone(item.entry, session.user), locale.locale) || item.entry.created_label }}
                <template v-if="item.entry.reactions.length">
                  · {{ item.entry.reactions.map((r) => `${r.emoji} ${r.count}`).join(" ") }}
                </template>
              </span>
              <label v-if="mine && item.entry.parent_id == null" class="feed-only-me fine">
                <input
                  type="checkbox"
                  :checked="item.entry.visibility === 'only_me'"
                  :disabled="switching.has(item.entry.id)"
                  @change="setOnlyMe(item, ($event.target as HTMLInputElement).checked)"
                />
                {{ t("diary.onlyMe") }}
              </label>
            </span>
          </article>
        </li>
      </ol>
      <button
        v-if="hasMore"
        class="text-btn feed-load-more"
        type="button"
        :disabled="busy"
        @click="load(true)"
      >
        {{ t("common.showMore") }}
      </button>
    </template>
  </section>
</template>

<style scoped>
.diary-feed {
  display: grid;
  gap: var(--space-2);
}

.feed-skeleton {
  height: 130px;
}

.feed-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.feed-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.feed-cover {
  flex-shrink: 0;
}

.feed-empty {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.feed-meta {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.feed-head {
  color: inherit;
  text-decoration: none;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-2);
}

.feed-body {
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.feed-body .badge {
  margin-right: var(--space-1);
  vertical-align: middle;
}

.feed-body.shielded {
  font-style: italic;
}

.feed-body.expanded {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.feed-more,
.feed-load-more {
  justify-self: start;
  font-size: var(--text-sm);
}

.feed-foot {
  margin-top: 2px;
}

.feed-club {
  color: inherit;
}

.feed-only-me {
  justify-self: start;
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: var(--tap);
}
</style>
