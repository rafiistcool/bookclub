<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { activityLine } from "../activity";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { isClubGone, useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import { postTimeZone } from "../timezone";
import type { ActivityItem } from "../types";
import Avatar from "./Avatar.vue";
import BookCover from "./BookCover.vue";
import RelativeTime from "./RelativeTime.vue";

/**
 * What happened: picks, votes, notes, shelves. Without `club` it is the home
 * feed across every club of the viewer (plus their own events); with a slug,
 * that club's feed.
 */
const props = defineProps<{
  club?: string;
  heading?: string;
  emptyText?: string;
  pageSize?: number;
  /** A short preview (Home): this many rows a page instead of `pageSize`. */
  limit?: number;
  /** Where the whole feed is: a "See all" link there replaces "Show more". */
  moreTo?: string;
}>();

const { t } = useI18n();
const session = useSession();
const clubs = useClubs();

const items = ref<ActivityItem[]>([]);
const error = ref("");
const loaded = ref(false);
const hasMore = ref(false);
const busy = ref(false);
/** The answer's zone: the club's in a club feed, where every line is in it (personal events too). */
const feedZone = ref<string | null>(null);

const rows = computed(() => items.value.map((item) => ({ item, line: activityLine(item) })));

async function load(more = false) {
  if (busy.value) return;
  busy.value = true;
  try {
    const before = more ? items.value[items.value.length - 1]?.id : undefined;
    const page = await api.activity({ before, limit: props.limit ?? props.pageSize ?? 15, club: props.club });
    items.value = more ? [...items.value, ...page.items] : page.items;
    hasMore.value = page.has_more;
    feedZone.value = page.timezone;
    error.value = "";
  } catch (err) {
    if (props.club && isClubGone(err)) clubs.markGone(props.club);
    error.value = err instanceof ApiError ? err.message : t("activity.loadFailed");
  } finally {
    loaded.value = true;
    busy.value = false;
  }
}

/**
 * A club feed shows every line in the club's zone, as its labels are; the home
 * feed shows club events in their club's zone and personal ones in the viewer's.
 */
function zoneOf(item: ActivityItem): string {
  return props.club && feedZone.value ? feedZone.value : postTimeZone(item, session.user);
}

onMounted(() => void load());
watch(
  () => props.club,
  () => {
    loaded.value = false;
    items.value = [];
    void load();
  },
);
</script>

<template>
  <section class="activity">
    <div class="section-head">
      <h2>{{ heading ?? t("activity.heading") }}</h2>
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
    <p v-else-if="items.length === 0" class="fine subtle">{{ emptyText ?? t("activity.empty") }}</p>
    <template v-else>
      <ol class="activity-list">
        <li v-for="{ item, line } in rows" :key="item.id" class="activity-row">
          <RouterLink
            v-if="item.book"
            class="activity-cover"
            :to="bookPath(item.book)"
            tabindex="-1"
            aria-hidden="true"
          >
            <BookCover :book="item.book" size="xs" />
          </RouterLink>
          <Avatar v-else :username="item.actor ?? ''" size="sm" />
          <span class="activity-meta">
            <i18n-t :keypath="line.key" tag="span" class="activity-text" scope="global">
              <template #actor>
                <RouterLink v-if="item.actor" class="activity-actor" :to="`/u/${item.actor}`">{{ item.actor }}</RouterLink>
                <span v-else>{{ t("activity.someone") }}</span>
              </template>
              <template #book>
                <RouterLink v-if="item.book" class="activity-book" :to="bookPath(item.book)">{{ item.book.title }}</RouterLink>
              </template>
              <template v-for="(value, name) in line.params" :key="name" #[name]>{{ value }}</template>
            </i18n-t>
            <span v-if="line.detail" class="activity-detail clamp-2">{{ line.detail }}</span>
            <span class="finer subtle">
              <template v-if="!club && item.club">
                <RouterLink class="activity-club" :to="`/c/${item.club.slug}`">{{ item.club.name }}</RouterLink> ·
              </template>
              <RelativeTime :iso="item.created_at" :zone="zoneOf(item)" :fallback="item.created_label" />
            </span>
          </span>
        </li>
      </ol>
      <RouterLink v-if="hasMore && moreTo" class="activity-all" :to="moreTo">{{ t("home.seeAll") }}</RouterLink>
      <button v-else-if="hasMore" class="text-btn activity-more" type="button" :disabled="busy" @click="load(true)">
        {{ t("common.showMore") }}
      </button>
    </template>
  </section>
</template>

<style scoped>
.activity {
  display: grid;
  gap: var(--space-2);
}

.activity-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
}

/* Light rows: no card, a hairline between one event and the next. */
.activity-row + .activity-row {
  border-top: 1px solid var(--border);
}

.activity-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3) 0;
}

.activity-cover {
  flex-shrink: 0;
}

.activity-meta {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

/* The same scale as the diary feed beside it: small text, names and
   titles set off by weight rather than underlines. */
.activity-text {
  font-size: var(--text-sm);
  line-height: var(--leading-snug);
  overflow-wrap: anywhere;
}

.activity-actor,
.activity-book {
  color: inherit;
  font-weight: 600;
  text-decoration: none;
}

.activity-book {
  font-family: var(--serif);
}

@media (hover: hover) {
  .activity-actor:hover,
  .activity-book:hover {
    text-decoration: underline;
  }
}

.activity-actor:active,
.activity-book:active {
  opacity: 0.6;
}

.activity-detail {
  color: var(--text-muted);
  font-size: var(--text-sm);
  overflow-wrap: anywhere;
}

.activity-club {
  color: inherit;
}

/* Its text lines up with the rows; the padding only widens the target. */
.activity-more {
  justify-self: start;
  margin-inline-start: calc(-1 * var(--space-2));
  font-size: var(--text-sm);
}

.activity-all {
  justify-self: start;
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  font-size: var(--text-sm);
  font-weight: 600;
}
</style>
