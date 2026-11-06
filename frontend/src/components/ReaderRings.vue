<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { statusShort, type Status } from "../constants";
import Avatar from "./Avatar.vue";

/**
 * Where everyone is in one book: each reader's avatar inside a ring that
 * fills with their progress (full, with a check, once finished). `md` puts
 * the names below; members who have not started yet (`memberCount` minus the
 * readers) get one dashed ring with their count. The rings fill once when
 * they appear, on `--ease-out`; under reduced motion they are simply full.
 */
export type RingReader = {
  username: string;
  avatar_url: string | null;
  progress: number | null;
  status: string;
};

const props = withDefaults(
  defineProps<{
    readers: RingReader[];
    memberCount?: number;
    size?: "sm" | "md";
    max?: number;
  }>(),
  { memberCount: undefined, size: "md", max: 6 },
);

const { t } = useI18n();

/** 0–100: the share of the ring that is drawn. */
function fill(reader: RingReader): number {
  if (reader.status === "finished") return 100;
  if (reader.status !== "currently_reading") return 0;
  return Math.min(100, Math.max(0, Math.round(reader.progress ?? 0)));
}

function label(reader: RingReader): string {
  if (reader.status === "finished") return t("rings.finished", { name: reader.username });
  if (reader.status === "currently_reading") return t("rings.reading", { name: reader.username, n: fill(reader) });
  const known = ["want_to_read", "did_not_finish"].includes(reader.status);
  return t("rings.status", {
    name: reader.username,
    status: known ? statusShort(reader.status as Status) : reader.status,
  });
}

const shown = computed(() =>
  props.readers.slice(0, Math.max(0, props.max)).map((reader) => ({
    reader,
    fill: fill(reader),
    label: label(reader),
  })),
);
const overflow = computed(() => Math.max(0, props.readers.length - shown.value.length));
const notStarted = computed(() =>
  props.memberCount == null ? 0 : Math.max(0, props.memberCount - props.readers.length),
);
</script>

<template>
  <ul class="reader-rings" :class="`size-${size}`" :aria-label="t('home.whereEveryone')">
    <li v-for="row in shown" :key="row.reader.username" class="ring-item" :class="row.reader.status">
      <span class="visually-hidden">{{ row.label }}</span>
      <span class="ring" aria-hidden="true">
        <svg class="ring-svg" viewBox="0 0 44 44" focusable="false">
          <circle class="ring-track" cx="22" cy="22" r="20" />
          <circle
            v-if="row.fill > 0"
            class="ring-fill"
            cx="22"
            cy="22"
            r="20"
            pathLength="100"
            :style="{ strokeDashoffset: 100 - row.fill }"
          />
        </svg>
        <Avatar class="ring-avatar" :username="row.reader.username" :src="row.reader.avatar_url" :size="size" />
        <span v-if="row.reader.status === 'finished'" class="ring-check">
          <svg viewBox="0 0 12 12" focusable="false"><path d="M2.5 6.2 5 8.6 9.6 3.6" /></svg>
        </span>
      </span>
      <span v-if="size === 'md'" class="ring-name" aria-hidden="true">{{ row.reader.username }}</span>
    </li>
    <li v-if="overflow" class="ring-item ring-overflow">
      <span class="visually-hidden">{{ t("rings.more", { n: overflow }) }}</span>
      <span class="ring ring-count nums" aria-hidden="true">+{{ overflow }}</span>
      <span v-if="size === 'md'" class="ring-name" aria-hidden="true">&nbsp;</span>
    </li>
    <li v-if="notStarted" class="ring-item ring-waiting">
      <span class="visually-hidden">{{ t("rings.notStarted", { n: notStarted }) }}</span>
      <span class="ring ring-count nums" aria-hidden="true">{{ notStarted }}</span>
      <span v-if="size === 'md'" class="ring-name" aria-hidden="true">{{ t("rings.notYet") }}</span>
    </li>
  </ul>
</template>

<style scoped>
.reader-rings {
  --ring: 44px;
  --ring-stroke: 3px;
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  /* Six 48px items fit one row at 360px: four faces, "+n" and "not yet". */
  gap: var(--space-2);
}

.reader-rings.size-sm {
  --ring: 32px;
  --ring-stroke: 3.5px;
  gap: var(--space-1);
}

.ring-item {
  display: grid;
  justify-items: center;
  gap: var(--space-1);
  min-width: var(--ring);
}

.size-md .ring-item {
  width: 48px;
}

.ring {
  position: relative;
  display: grid;
  place-items: center;
  width: var(--ring);
  height: var(--ring);
}

.ring-svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  transform: rotate(-90deg);
  overflow: visible;
}

.ring-track,
.ring-fill {
  fill: none;
  stroke-width: var(--ring-stroke);
}

.ring-track {
  stroke: var(--surface-3);
}

.ring-fill {
  stroke: var(--accent);
  stroke-linecap: round;
  stroke-dasharray: 100;
  /* Fills once from empty when the rings appear; later changes just land. */
  animation: ring-fill calc(var(--dur-slow) * 2.4) var(--ease-out) both;
}

@keyframes ring-fill {
  from {
    stroke-dashoffset: 100;
  }
}

.ring-item.want_to_read .ring-avatar,
.ring-item.did_not_finish .ring-avatar {
  opacity: 0.6;
}

.ring-check {
  position: absolute;
  right: -2px;
  bottom: -2px;
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  box-shadow: 0 0 0 2px var(--bg);
}

.size-sm .ring-check {
  width: 13px;
  height: 13px;
}

.ring-check svg {
  width: 70%;
  height: 70%;
  fill: none;
  stroke: var(--accent-ink);
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.ring-count {
  border-radius: var(--radius-pill);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: 700;
}

.ring-waiting .ring-count {
  background: transparent;
  border: 1.5px dashed var(--border-strong);
}

.ring-name {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-muted);
  font-size: var(--text-xs);
  line-height: 1.2;
}

@media (prefers-reduced-motion: reduce) {
  .ring-fill {
    animation: none;
  }
}
</style>
