<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { vAutosize } from "../autosize";
import { tp } from "../i18n";
import { momentPatch, useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import BookCover from "./BookCover.vue";
import BottomSheet from "./BottomSheet.vue";

/**
 * The moment a book is finished (`stores/finish.ts` opens it; App.vue mounts
 * it). The status is already saved when it opens, so closing it loses
 * nothing but the stars. "Done" saves the stars, the line and "only me" in
 * one PATCH, and only what was set; "Skip", Escape and a drag close it.
 *
 * The one authored motion is the check drawn at the cover's corner. Under
 * reduced motion everything stands at its end state: the check drawn, the
 * stars filled at once.
 */
const { t } = useI18n();
const finish = useFinish();
const session = useSession();

// Mounted only while a moment is open (App.vue's v-if); the entry is fixed for its life.
const item = finish.item!;
const STAGGER_MS = 30;

const rating = ref<number | null>(item.rating);
/** How many stars show filled: it follows `rating` one star at a time. */
const lit = ref(item.rating ?? 0);
const takeOpen = ref(Boolean(item.take));
const take = ref(item.take);
const onlyMe = ref(item.rating_visibility === "only_me");
const busy = ref(false);
const error = ref("");
const takeField = ref<HTMLTextAreaElement | null>(null);
const club = ref<{ finished: number; total: number } | null>(null);

let stagger: ReturnType<typeof setTimeout> | null = null;

const clubLine = computed(() => {
  const counts = club.value;
  if (!counts || counts.total < 2) return "";
  if (counts.finished >= counts.total) return t("finish.clubAll", { total: counts.total });
  return tp("finish.clubLine", counts.finished, { total: counts.total });
});

function reducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function stopStagger() {
  if (stagger !== null) clearTimeout(stagger);
  stagger = null;
}

function advance() {
  stagger = null;
  const target = rating.value ?? 0;
  if (lit.value === target) return;
  lit.value += lit.value < target ? 1 : -1;
  if (lit.value !== target) stagger = setTimeout(advance, STAGGER_MS);
}

/** A tap fills the stars up to it, one after another. Tapping the chosen star again keeps it. */
function rate(value: number) {
  if (typeof navigator !== "undefined" && typeof navigator.vibrate === "function") navigator.vibrate(6);
  if (rating.value === value) return;
  rating.value = value;
  stopStagger();
  if (reducedMotion()) lit.value = value;
  else advance();
}

async function openTake() {
  takeOpen.value = true;
  await nextTick();
  takeField.value?.focus();
}

/** "2 of 4 in the club are done": only for the club's current pick, and silently nothing when that is unknown. */
async function loadClub() {
  const slug = finish.clubSlug;
  const summary = slug ? session.user?.clubs.find((row) => row.slug === slug) : undefined;
  if (!slug || !summary) return;
  try {
    const { pick } = await api.clubPick(slug);
    if (!pick || pick.book.id !== item.book.id) return;
    const me = session.user?.username;
    // The status is saved before the moment opens; count me even if the answer lags.
    const finished = pick.finished.length + (me && !pick.finished.includes(me) ? 1 : 0);
    club.value = { finished: Math.min(finished, summary.member_count), total: summary.member_count };
  } catch {
    /* The line is a nicety; without it the moment is complete. */
  }
}

async function done() {
  if (busy.value) return;
  const body = momentPatch(item, { rating: rating.value, take: take.value, onlyMe: onlyMe.value });
  if (!body) {
    finish.close(null);
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    finish.close(await api.patchShelf(item.id, body));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("book.saveFailed");
  } finally {
    busy.value = false;
  }
}

function skip() {
  if (!busy.value) finish.close(null);
}

onMounted(() => void loadClub());
onBeforeUnmount(stopStagger);
</script>

<template>
  <BottomSheet class="finished-moment" :title="t('finish.title')" @close="skip">
    <div class="moment-book">
      <div class="moment-cover">
        <BookCover :book="item.book" size="lg" eager />
        <svg class="moment-check" viewBox="0 0 24 24" width="36" height="36" aria-hidden="true">
          <circle cx="12" cy="12" r="11" />
          <path d="M7 12.5 10.5 16 17 8.5" pathLength="1" />
        </svg>
      </div>
      <p class="moment-title">{{ item.book.title }}</p>
      <p v-if="item.book.authors" class="moment-authors">{{ item.book.authors }}</p>
    </div>

    <div class="moment-stars" role="group" :aria-label="t('finish.rate')">
      <button
        v-for="value in 5"
        :key="value"
        class="moment-star"
        :class="{ lit: value <= lit }"
        type="button"
        :aria-label="t('book.starsOf', { n: value })"
        :aria-pressed="rating === value"
        :disabled="busy"
        @click="rate(value)"
      >
        <svg viewBox="0 0 24 24" width="32" height="32" aria-hidden="true">
          <path d="M12 3.2l2.7 5.5 6 .9-4.35 4.25 1.03 6L12 17l-5.38 2.85 1.03-6L3.3 9.6l6-.9z" />
        </svg>
      </button>
    </div>

    <div class="moment-rows">
      <button v-if="!takeOpen" class="text-btn moment-add-take" type="button" :disabled="busy" @click="openTake">
        {{ t("finish.addTake") }}
      </button>
      <label v-else class="field moment-take">
        <span class="field-head">{{ t("book.oneLineTake") }}<span class="finer subtle nums">{{ take.length }}/140</span></span>
        <textarea
          ref="takeField"
          v-model="take"
          v-autosize
          rows="1"
          maxlength="140"
          :disabled="busy"
          :placeholder="t('book.takePlaceholder')"
        />
      </label>

      <label class="moment-only-me">
        <input v-model="onlyMe" type="checkbox" :disabled="busy" />
        <span>{{ t("finish.onlyMe") }}</span>
      </label>

      <p v-if="clubLine" class="moment-club">{{ clubLine }}</p>
    </div>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <div class="moment-actions">
      <button class="btn btn-primary btn-block" type="button" :disabled="busy" @click="done">
        {{ t("finish.done") }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="skip">
        {{ t("finish.skip") }}
      </button>
    </div>
  </BottomSheet>
</template>

<style scoped>
/* The sheet's own title is the moment's headline, set as a display heading. */
.finished-moment :deep(.sheet-head h2) {
  font-family: var(--serif);
  font-size: var(--text-3xl);
  line-height: var(--leading-tight);
  letter-spacing: var(--tracking-tight);
}

.moment-book {
  display: grid;
  justify-items: center;
  gap: var(--space-1);
  margin: var(--space-2) 0 var(--space-4);
  text-align: center;
}

/* The book lifts into the hand. */
.moment-cover {
  position: relative;
  width: 104px;
  margin-bottom: var(--space-2);
  animation: cover-in var(--dur-slow) var(--ease-out) 60ms both;
}

.moment-cover :deep(.cover) {
  box-shadow: var(--elev-float);
}

/* The authored moment: a check drawn at the cover's corner once the book has landed. */
.moment-check {
  position: absolute;
  top: -12px;
  right: -14px;
  filter: drop-shadow(0 1px 2px rgb(0 0 0 / 0.18));
}

.moment-check circle {
  fill: var(--accent);
  stroke: var(--bg);
  stroke-width: 2;
}

.moment-check path {
  fill: none;
  stroke: var(--accent-ink);
  stroke-width: 2.4;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-dasharray: 1;
  stroke-dashoffset: 0;
  animation: check-draw 380ms var(--ease-out) 320ms both;
}

.moment-title {
  max-width: 28ch;
  font-family: var(--serif);
  font-size: var(--text-lg);
  line-height: var(--leading-tight);
  overflow-wrap: anywhere;
}

.moment-authors {
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.moment-stars {
  display: flex;
  justify-content: center;
  gap: var(--space-1);
  margin-bottom: var(--space-3);
}

.moment-star {
  display: grid;
  place-items: center;
  width: var(--tap);
  height: var(--tap);
  padding: 0;
  border: 0;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--border-strong);
  transition: transform var(--dur-fast) var(--ease-out);
}

.moment-star:active:not(:disabled) {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.moment-star path {
  fill: transparent;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linejoin: round;
  transition:
    fill var(--dur-fast) var(--ease-out),
    stroke var(--dur-fast) var(--ease-out);
}

.moment-star.lit {
  color: var(--accent);
}

.moment-star.lit path {
  fill: var(--accent);
}

.moment-star:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

/* Light rows: a hairline between them, no boxes. */
.moment-rows {
  display: grid;
  margin-bottom: var(--space-4);
  border-top: 1px solid var(--border);
}

.moment-rows > * {
  border-bottom: 1px solid var(--border);
}

.moment-add-take {
  justify-self: stretch;
  text-align: left;
  padding: 0;
}

.moment-take {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: var(--space-3) 0;
}

.field-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.field-head .finer {
  font-weight: 400;
}

.moment-only-me {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  min-height: var(--tap);
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.moment-only-me input {
  order: 2;
  width: 20px;
  height: 20px;
  accent-color: var(--accent);
}

.moment-club {
  display: flex;
  align-items: center;
  min-height: var(--tap);
  color: var(--text-muted);
  font-size: var(--text-sm);
  animation: line-in var(--dur-base) var(--ease-out) both;
}

.moment-actions {
  display: grid;
  gap: var(--space-2);
}

@keyframes cover-in {
  from {
    opacity: 0;
    transform: translateY(12px) scale(0.92);
  }
}

@keyframes check-draw {
  from {
    stroke-dashoffset: 1;
  }
}

@keyframes line-in {
  from {
    opacity: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .moment-cover,
  .moment-check path,
  .moment-club {
    animation: none;
  }

  .moment-star:active:not(:disabled) {
    transform: none;
  }
}
</style>
