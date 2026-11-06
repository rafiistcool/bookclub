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
import { computed, onBeforeMount, onBeforeUnmount, ref as localRef } from "vue";
import { formatClubDate, formatRelative } from "../i18n/dates";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";

/**
 * When something was written, relative to now ("vor 5 Min.", "gestern"). Days
 * are counted in the viewer's own zone, so "gestern" means the reader's
 * yesterday and a feed reads in order; `zone` (the row's zone, e.g. the
 * club's) is the zone of the exact time in `title` and on a tap, which shows
 * it in place for touch screens without hover. `fallback` (the server's
 * label) shows when `iso` cannot be read.
 */
const props = defineProps<{
  iso: string | null | undefined;
  zone: string;
  fallback?: string;
}>();

const locale = useLocale();
const session = useSession();
const showExact = localRef(false);

// The account's zone when it has one; otherwise the row's, as before.
const viewerZone = computed(() => session.user?.timezone || props.zone);
const label = computed(() => formatRelative(props.iso, viewerZone.value, locale.locale, now.value));
const exact = computed(() => formatClubDate(props.iso, props.zone, locale.locale));

onBeforeMount(join);
onBeforeUnmount(leave);
</script>

<template>
  <time
    v-if="label"
    class="relative-time"
    :datetime="iso ?? undefined"
    :title="exact || undefined"
    @click.stop="showExact = !showExact"
  >{{ showExact && exact ? exact : label }}</time>
  <span v-else>{{ exact || fallback }}</span>
</template>

<style scoped>
/* A tap shows the exact time in place (there is no hover on a phone). */
.relative-time {
  cursor: pointer;
  -webkit-user-select: none;
  user-select: none;
}
</style>
