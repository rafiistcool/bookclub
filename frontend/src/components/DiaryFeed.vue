<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { clubFeedPreview, EXCERPT_LENGTH, excerpt, positionMarker } from "../diary";
import { isGuarded, spoilerGuard } from "../spoilers";
import { isClubGone, useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { postTimeZone } from "../timezone";
import type { DiaryFeedItem } from "../types";
import BookCover from "./BookCover.vue";
import RelativeTime from "./RelativeTime.vue";

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
  /** Keep the heading for screen readers only, under a page title that
      already says it (`/diary`). */
  headingHidden?: boolean;
  /** Shown when nothing is written yet. */
  emptyText?: string;
  pageSize?: number;
  /** A short preview (Home): this many rows a page instead of `pageSize`. */
  limit?: number;
  /** Where the whole feed is: a "See all" link there replaces "Show more". */
  moreTo?: string;
  /** With `club`: the work of the pick thread the page already shows above
      (`/c/:slug/discussions`). That club's posts on it are left out here. */
  threadWork?: number | null;
}>();

const { t } = useI18n();
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
/** A reload asked for while a page was loading: that page may predate the change, so read again. */
let reloadAgain = false;

async function load(more = false) {
  if (busy.value) return;
  busy.value = true;
  try {
    let before = more ? cursor.value : undefined;
    const shown: DiaryFeedItem[] = [];
    let page: Awaited<ReturnType<typeof api.diaryFeed>>;
    // A page made only of left-out thread posts would show nothing new, so
    // read on (a few pages at most) until something is left to show.
    for (let tries = 0; ; tries++) {
      page = props.mine
        ? await api.diaryFeed(before, props.limit ?? props.pageSize ?? 10, undefined, true)
        : await api.diaryFeed(before, props.limit ?? props.pageSize ?? 8, props.club);
      shown.push(...page.items.filter((item) => !inThread(item)));
      before = page.items[page.items.length - 1]?.entry.id ?? before;
      if (shown.length > 0 || !page.has_more || tries >= 4) break;
    }
    feed.value = more ? [...feed.value, ...shown] : shown;
    hasMore.value = page.has_more;
    cursor.value = before;
    error.value = "";
  } catch (err) {
    if (props.club && isClubGone(err)) clubs.markGone(props.club);
    error.value = err instanceof ApiError ? err.message : t("club.diaryLoadFailed");
  } finally {
    loaded.value = true;
    busy.value = false;
    if (reloadAgain) {
      reloadAgain = false;
      void load();
    }
  }
}

/** The first page again: the viewer's position moved, so the spoiler guards (`my_progress`) did. */
function reload(): Promise<void> {
  if (busy.value) {
    reloadAgain = true;
    return Promise.resolve();
  }
  return load();
}

/** A post of the thread the page already shows (`threadWork`). */
function inThread(item: DiaryFeedItem): boolean {
  return props.threadWork != null && item.club?.slug === props.club && item.book.id === props.threadWork;
}

const expanded = ref<Set<number>>(new Set());
/** Posts the reader tapped open although they are not far enough (`spoilers.ts`), for this view. */
const revealed = ref<Set<number>>(new Set());

function guarded(item: DiaryFeedItem): boolean {
  return !revealed.value.has(item.entry.id) && isGuarded(item.entry, item.my_progress, item.my_status);
}

/** The foot's position marker, unless the folded line already starts with it (an unflagged post written further on). */
function footMarker(item: DiaryFeedItem): string {
  if (guarded(item) && spoilerGuard(item.entry, item.my_progress, item.my_status).kind === "ahead") return "";
  return positionMarker(item.entry);
}

function reveal(id: number) {
  revealed.value = new Set([...revealed.value, id]);
}

function preview(item: DiaryFeedItem): string {
  return revealed.value.has(item.entry.id) ? excerpt(item.entry.body) : clubFeedPreview(item);
}
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
  if (item.entry.deleted || guarded(item)) return false;
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
defineExpose({ reload });
</script>

<template>
  <section class="diary-feed">
    <div class="section-head">
      <h2 :class="{ 'visually-hidden': headingHidden }">{{ heading ?? t("club.recentlyWritten") }}</h2>
      <slot name="head" />
    </div>
    <p v-if="error" class="error">{{ error }}</p>
    <ol v-else-if="!loaded" class="skeleton-rows" aria-hidden="true">
      <li v-for="n in 3" :key="n" class="skeleton-row">
        <span class="skeleton skeleton-cover-xs" />
        <span class="skeleton-row-lines">
          <span class="skeleton skeleton-line short" />
          <span class="skeleton skeleton-line" />
        </span>
      </li>
    </ol>
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
                <strong class="feed-title">{{ item.book.title }}</strong>
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
              <!-- Not far enough for it: one quiet line instead of the text, a tap shows it here. -->
              <button
                v-if="guarded(item)"
                class="feed-guard"
                type="button"
                @click="reveal(item.entry.id)"
              >
                <span class="feed-guard-line">{{ clubFeedPreview(item) }}</span>
                <span class="feed-guard-hint">{{ t("spoilers.tapToRead") }}</span>
              </button>
              <span
                v-else
                :ref="(el) => measureBody(item.entry.id, el as HTMLElement | null)"
                class="feed-body"
                :class="{
                  'clamp-2': !expanded.has(item.entry.id),
                  expanded: expanded.has(item.entry.id),
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
                {{ expanded.has(item.entry.id) ? item.entry.body.trim() : preview(item) }}
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
              <span class="feed-foot-row">
                <span class="finer subtle feed-foot">
                  <template v-if="!club && item.club">
                    <RouterLink class="feed-club" :to="`/c/${item.club.slug}`">{{ item.club.name }}</RouterLink> ·
                  </template>
                  <template v-if="footMarker(item)">
                    {{ footMarker(item) }} ·
                  </template>
                  <RelativeTime
                    :iso="item.entry.created_at"
                    :zone="postTimeZone(item.entry, session.user)"
                    :fallback="item.entry.created_label"
                  />
                  <template v-if="item.entry.reactions.length">
                    ·
                    <span v-for="r in item.entry.reactions" :key="r.emoji" class="feed-reaction">{{ r.emoji }}&nbsp;{{ r.count }}</span>
                  </template>
                </span>
                <!-- A quiet switch at the end of the foot line, not a line of its own. -->
                <label v-if="mine && item.entry.parent_id == null" class="feed-only-me finer">
                  <input
                    type="checkbox"
                    :checked="item.entry.visibility === 'only_me'"
                    :disabled="switching.has(item.entry.id)"
                    @change="setOnlyMe(item, ($event.target as HTMLInputElement).checked)"
                  />
                  {{ t("diary.onlyMe") }}
                </label>
              </span>
            </span>
          </article>
        </li>
      </ol>
      <RouterLink v-if="hasMore && moreTo" class="feed-all" :to="moreTo">{{ t("home.seeAll") }}</RouterLink>
      <button
        v-else-if="hasMore"
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

.feed-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
}

/* Light rows: no card, a hairline between one note and the next. */
.feed-list > li + li {
  border-top: 1px solid var(--border);
}

.feed-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3) 0;
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

/* A finger high: the hit area reaches 44 px around the one-line head (the
   pattern of .chip::before), into the row's padding and the note's first line. */
.feed-head {
  position: relative;
  color: inherit;
  text-decoration: none;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-2);
}

.feed-head::before {
  content: "";
  position: absolute;
  inset: min(0px, calc((100% - var(--tap)) / 2)) 0;
}

.feed-body {
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.feed-body .badge {
  margin-right: var(--space-1);
  vertical-align: middle;
  font-style: normal;
}

.feed-guard {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 var(--space-2);
  min-height: var(--tap);
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--text-sm);
  text-align: left;
  align-content: center;
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-out);
}

/* "bei 35 % · …" starts the line, so it starts with a capital. */
.feed-guard:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (prefers-reduced-motion: reduce) {
  .feed-guard:active {
    transform: none;
  }
}

@media (hover: hover) {
  .feed-guard:hover .feed-guard-line {
    color: var(--text);
  }
}

.feed-guard-line {
  display: inline-block;
}

.feed-guard-line::first-letter {
  text-transform: uppercase;
}

.feed-guard-hint {
  color: var(--accent-hover);
  font-weight: 600;
  white-space: nowrap;
}

.feed-body.expanded {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* Their text lines up with the rows; the padding only widens the target. */
.feed-more,
.feed-load-more {
  justify-self: start;
  margin-inline-start: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
}

/* Book titles are serif everywhere. */
.feed-title {
  font-family: var(--serif);
  font-weight: 600;
}

.feed-all {
  justify-self: start;
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  font-size: var(--text-sm);
  font-weight: 600;
}

.feed-foot-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0 var(--space-3);
  margin-top: 2px;
}

/* An emoji never leaves its count alone on the next line. */
.feed-reaction {
  white-space: nowrap;
}

.feed-reaction + .feed-reaction {
  margin-left: var(--space-1);
}

.feed-club {
  color: inherit;
}

/* 44px tall for the finger, but pulled into the foot line so it adds none. */
.feed-only-me {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--tap);
  margin-block: calc(-1 * var(--space-3));
  color: var(--text-muted);
  font-weight: 600;
  cursor: pointer;
}

.feed-only-me input {
  width: 18px;
  height: 18px;
  margin: 0;
  accent-color: var(--accent);
}
</style>
