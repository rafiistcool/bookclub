<script lang="ts">
import { ref } from "vue";

/** How often the shared clock moves: the labels' finest step is a minute. */
export const TICK_MS = 60_000;

/**
 * One clock for every mounted RelativeTime, so "vor 5 Min." does not go stale.
 * It runs one interval while at least one instance is mounted, pauses while the
 * page is hidden and catches up when it is shown again.
 */
const now = ref(Date.now());
let mounted = 0;
let timer: ReturnType<typeof setInterval> | null = null;

function tick() {
  now.value = Date.now();
}

function startTimer() {
  if (timer == null) timer = setInterval(tick, TICK_MS);
}

function stopTimer() {
  if (timer != null) clearInterval(timer);
  timer = null;
}

function onVisibility() {
  if (document.hidden) {
    stopTimer();
  } else {
    tick();
    startTimer();
  }
}

function join() {
  // A clock left from earlier mounts can be up to a tick old.
  if (Math.abs(Date.now() - now.value) >= 1000) tick();
  if (mounted++ > 0) return;
  document.addEventListener("visibilitychange", onVisibility);
  if (!document.hidden) startTimer();
}

function leave() {
  if (--mounted > 0) return;
  document.removeEventListener("visibilitychange", onVisibility);
  stopTimer();
}
</script>

<script setup lang="ts">
import { computed, onBeforeMount, onBeforeUnmount } from "vue";
import { formatClubDate, formatRelative } from "../i18n/dates";
import { useLocale } from "../stores/locale";

/**
 * When something was written, relative to now ("vor 5 Min.", "gestern"), with
 * the exact time in `title` and `datetime`. `zone` is the zone the row counts
 * days in; `fallback` (the server's label) shows when `iso` cannot be read.
 */
const props = defineProps<{
  iso: string | null | undefined;
  zone: string;
  fallback?: string;
}>();

const locale = useLocale();

const label = computed(() => formatRelative(props.iso, props.zone, locale.locale, now.value));
const exact = computed(() => formatClubDate(props.iso, props.zone, locale.locale));

onBeforeMount(join);
onBeforeUnmount(leave);
</script>

<template>
  <time v-if="label" :datetime="iso ?? undefined" :title="exact || undefined">{{ label }}</time>
  <span v-else>{{ exact || fallback }}</span>
</template>
