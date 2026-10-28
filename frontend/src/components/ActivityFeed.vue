<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { activityLine } from "../activity";
import { api, ApiError } from "../api/client";
import { bookPath } from "../constants";
import { formatClubDate } from "../i18n/dates";
import { isClubGone, useClubs } from "../stores/clubs";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { postTimeZone } from "../timezone";
import type { ActivityItem } from "../types";
import Avatar from "./Avatar.vue";
import BookCover from "./BookCover.vue";

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
}>();

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const clubs = useClubs();

const items = ref<ActivityItem[]>([]);
const error = ref("");
const loaded = ref(false);
const hasMore = ref(false);
const busy = ref(false);

const rows = computed(() => items.value.map((item) => ({ item, line: activityLine(item) })));

async function load(more = false) {
  if (busy.value) return;
  busy.value = true;
  try {
    const before = more ? items.value[items.value.length - 1]?.id : undefined;
    const page = await api.activity({ before, limit: props.pageSize ?? 15, club: props.club });
    items.value = more ? [...items.value, ...page.items] : page.items;
    hasMore.value = page.has_more;
    error.value = "";
  } catch (err) {
    if (props.club && isClubGone(err)) clubs.markGone(props.club);
    error.value = err instanceof ApiError ? err.message : t("activity.loadFailed");
  } finally {
    loaded.value = true;
    busy.value = false;
  }
}

function when(item: ActivityItem): string {
  return formatClubDate(item.created_at, postTimeZone(item, session.user), locale.locale) || item.created_label;
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
    <div v-else-if="!loaded" class="skeleton activity-skeleton" aria-hidden="true" />
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
              {{ when(item) }}
            </span>
          </span>
        </li>
      </ol>
      <button v-if="hasMore" class="text-btn activity-more" type="button" :disabled="busy" @click="load(true)">
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

.activity-skeleton {
  height: 130px;
}

.activity-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.activity-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
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

.activity-text {
  overflow-wrap: anywhere;
}

.activity-actor,
.activity-book {
  color: inherit;
  font-weight: 650;
}

.activity-book {
  font-family: var(--serif);
}

.activity-detail {
  color: var(--text-muted);
  font-size: var(--text-sm);
  overflow-wrap: anywhere;
}

.activity-club {
  color: inherit;
}

.activity-more {
  justify-self: start;
  font-size: var(--text-sm);
}
</style>
