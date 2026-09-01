<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import { STATUS_LABEL, STATUS_SHORT, formatDate, plural } from "../constants";
import { useFlow, toRef } from "../stores/flow";
import { usePick } from "../stores/pick";
import { useSession } from "../stores/session";
import { useShelf } from "../stores/shelf";
import type { BookDetails, BookRef, Quote } from "../types";
import Avatar from "./Avatar.vue";
import BookCover from "./BookCover.vue";
import NavIcon from "./NavIcon.vue";
import ProgressBar from "./ProgressBar.vue";
import Sheet from "./Sheet.vue";
import Skeleton from "./Skeleton.vue";
import StarRating from "./StarRating.vue";

const props = defineProps<{
  book: BookRef;
}>();

const emit = defineEmits<{
  close: [];
}>();

const flow = useFlow();
const shelf = useShelf();
const pick = usePick();
const session = useSession();

const details = ref<BookDetails | null>(null);
const quotes = ref<Quote[]>([]);
const error = ref("");
const expanded = ref(false);

const item = computed(() => shelf.byKey.get(props.book.ol_work_key) ?? null);
const isClubPick = computed(() => pick.pick?.book.ol_work_key === props.book.ol_work_key);
const title = computed(() => details.value?.title || props.book.title);
const authors = computed(() => details.value?.authors || props.book.authors);
const coverId = computed(() => details.value?.cover_id ?? props.book.cover_id);
const year = computed(() => details.value?.year ?? props.book.year);
const description = computed(() => details.value?.description ?? "");
const longDescription = computed(() => description.value.length > 420);
const shownDescription = computed(() =>
  expanded.value || !longDescription.value ? description.value : description.value.slice(0, 400).trimEnd() + "…",
);
const others = computed(() =>
  (details.value?.members ?? []).filter((row) => row.username !== session.user?.username),
);
const ref_ = computed<BookRef>(() => ({
  ol_work_key: props.book.ol_work_key,
  title: title.value,
  authors: authors.value,
  cover_id: coverId.value,
  year: year.value,
}));

async function load() {
  try {
    const [data, quoteList] = await Promise.all([
      api.bookDetails(props.book.ol_work_key),
      api.quotes({ work: props.book.ol_work_key, limit: 20 }),
    ]);
    details.value = data;
    quotes.value = quoteList.items;
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load this book";
  }
}

function openStatus() {
  flow.push({ kind: "status", book: ref_.value, item: item.value, title: item.value ? "Move to…" : "Add to shelf" });
}

onMounted(load);
</script>

<template>
  <Sheet :title="title" :subtitle="authors" @close="emit('close')">
    <div class="pick-hero" style="margin-bottom: 14px">
      <BookCover :title="title" :cover-id="coverId" size="lg" eager />
      <div class="meta">
        <div class="badges" style="display: flex; gap: 6px; flex-wrap: wrap">
          <span v-if="isClubPick" class="badge club">Club pick</span>
          <span v-if="item" class="badge" :class="item.status">{{ STATUS_LABEL[item.status] }}</span>
        </div>
        <p class="fine muted">
          <template v-if="year">{{ year }}</template>
          <template v-if="details?.pages"> · {{ details.pages }} pages</template>
        </p>
        <p v-if="details?.ol_rating" class="fine muted" style="display: flex; align-items: center; gap: 6px">
          <StarRating :value="Math.round(details.ol_rating)" />
          {{ details.ol_rating.toFixed(1) }}
          <span class="faint">Open Library · {{ plural(details.ol_rating_count ?? 0, "rating") }}</span>
        </p>
        <div v-if="item?.status === 'currently_reading'" style="margin-top: 4px">
          <ProgressBar :value="item.progress" thin />
        </div>
        <p v-if="item?.rating" class="fine" style="display: flex; align-items: center; gap: 6px">
          <StarRating :value="item.rating" /> <span class="muted">your rating</span>
        </p>
        <p v-if="item?.take" class="fine muted">“{{ item.take }}”</p>
      </div>
    </div>

    <div class="actions" style="margin-bottom: 16px">
      <button class="btn btn-primary" type="button" @click="openStatus">
        {{ item ? `Move from ${STATUS_SHORT[item.status]}…` : "Add to shelf" }}
      </button>
      <button
        v-if="item?.status === 'currently_reading'"
        class="btn btn-ghost"
        type="button"
        @click="flow.push({ kind: 'progress', item })"
      >
        Progress
      </button>
      <button class="btn btn-ghost" type="button" @click="flow.push({ kind: 'quote', book: ref_ })">
        <NavIcon name="quote" :size="18" /> Quote
      </button>
      <button
        v-if="!isClubPick"
        class="btn btn-ghost btn-sm"
        type="button"
        @click="flow.push({ kind: 'clubPick', book: ref_ })"
      >
        Set as club pick
      </button>
      <button v-if="!isClubPick" class="btn btn-ghost btn-sm" type="button" @click="flow.nominate(ref_)">
        Nominate
      </button>
      <button v-if="item" class="btn btn-danger btn-sm" type="button" @click="flow.push({ kind: 'remove', item })">
        Remove
      </button>
    </div>

    <p v-if="error" class="error fine">{{ error }}</p>
    <Skeleton v-else-if="!details" kind="lines" :count="4" />
    <template v-else>
      <section v-if="description">
        <p style="white-space: pre-wrap; overflow-wrap: anywhere">{{ shownDescription }}</p>
        <button v-if="longDescription" class="text-btn sm" type="button" @click="expanded = !expanded">
          {{ expanded ? "Show less" : "Read more" }}
        </button>
      </section>
      <p v-else class="muted fine">No synopsis on Open Library for this one.</p>

      <div v-if="details.subjects.length" class="chip-row" style="margin-top: 12px" aria-label="Subjects">
        <span v-for="subject in details.subjects" :key="subject" class="chip" style="min-height: 30px; font-weight: 500">
          {{ subject }}
        </span>
      </div>

      <section class="section" style="margin-top: 20px">
        <div class="section-title">
          <h2>In the club</h2>
        </div>
        <p v-if="others.length === 0" class="muted fine">Nobody else has this on their shelf yet.</p>
        <ul v-else class="readers" style="list-style: none; padding: 0">
          <li v-for="row in others" :key="row.username" class="reader-row">
            <Avatar :username="row.username" size="sm" />
            <div style="min-width: 0">
              <RouterLink class="name" :to="`/friends/${row.username}`" @click="emit('close')">{{ row.username }}</RouterLink>
              <span class="sub"> · {{ STATUS_LABEL[row.status] }}<template v-if="row.finished_at"> {{ formatDate(row.finished_at) }}</template></span>
              <div v-if="row.status === 'currently_reading' && row.progress != null" style="margin-top: 4px">
                <ProgressBar :value="row.progress" thin />
              </div>
            </div>
            <StarRating v-if="row.rating" :value="row.rating" />
            <p v-if="row.take" class="take">“{{ row.take }}”</p>
          </li>
        </ul>
      </section>

      <section v-if="quotes.length" class="section">
        <div class="section-title">
          <h2>Quotes</h2>
          <RouterLink class="fine" :to="`/quotes?work=${encodeURIComponent(book.ol_work_key)}`" @click="emit('close')">All</RouterLink>
        </div>
        <ul class="list" style="list-style: none; padding: 0">
          <li v-for="quote in quotes.slice(0, 3)" :key="quote.id" class="card">
            <p class="serif" style="font-style: italic">“{{ quote.body }}”</p>
            <p class="fine muted" style="margin-top: 6px">
              {{ quote.author }}<template v-if="quote.page"> · p. {{ quote.page }}</template>
            </p>
          </li>
        </ul>
      </section>

      <p class="fine" style="margin-top: 16px">
        <a :href="`https://openlibrary.org${book.ol_work_key}`" target="_blank" rel="noreferrer">Open Library ↗</a>
      </p>
    </template>
  </Sheet>
</template>
