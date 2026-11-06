<script setup lang="ts">
import { CheckIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "vue-i18n";
import { MODES, THEMES, useTheme } from "../../stores/theme";
import type { ColorMode, ThemeId } from "../../types";
import BottomSheet from "../BottomSheet.vue";

/**
 * Light, dark or the device's mode, and the four palettes. Each choice
 * applies at once, behind the sheet's shade, so the page shows it before the
 * sheet closes. Where the browser can, the old palette crossfades into the
 * new one instead of snapping.
 */
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const theme = useTheme();

type WithViewTransition = Document & { startViewTransition?: (update: () => void) => unknown };

function crossfade(update: () => void) {
  const doc = document as WithViewTransition;
  const still = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (still || typeof doc.startViewTransition !== "function") update();
  else doc.startViewTransition(update);
}

function pickMode(id: ColorMode) {
  if (id !== theme.mode) crossfade(() => theme.setMode(id));
}

function pickTheme(id: ThemeId) {
  if (id !== theme.theme) crossfade(() => theme.setTheme(id));
}
</script>

<template>
  <BottomSheet class="appearance-sheet" :title="t('settings.appearance')" @close="emit('close')">
    <div class="segmented mode-switch" role="group" :aria-label="t('settings.colourMode')">
      <button
        v-for="option in MODES"
        :key="option.id"
        type="button"
        :aria-pressed="theme.mode === option.id"
        @click="pickMode(option.id)"
      >
        {{ t(option.labelKey) }}
      </button>
    </div>
    <p class="fine subtle mode-now" :class="{ hidden: theme.mode !== 'system' }" :aria-hidden="theme.mode !== 'system'">
      {{ t("settings.followingDevice", { mode: t(`theme.${theme.resolvedMode}`) }) }}
    </p>

    <div class="swatches">
      <button
        v-for="option in THEMES"
        :key="option.id"
        class="swatch"
        :class="{ active: theme.theme === option.id }"
        type="button"
        :aria-pressed="theme.theme === option.id"
        @click="pickTheme(option.id)"
      >
        <!-- A miniature of the app in that palette: every value below
             resolves against the .pal-* class, not the active palette. -->
        <span class="swatch-preview" :class="`pal-${option.id}-${theme.resolvedMode}`" aria-hidden="true">
          <span class="sp-cover" />
          <span class="sp-text">
            <span class="sp-title" />
            <span class="sp-line" />
          </span>
          <span class="sp-accent" />
          <CheckIcon v-if="theme.theme === option.id" class="swatch-check" />
        </span>
        <span class="swatch-name">{{ t(option.labelKey) }}</span>
      </button>
    </div>
  </BottomSheet>
</template>

<style scoped>
.mode-switch {
  width: 100%;
}

.mode-now {
  min-height: 1.5em;
  margin: var(--space-2) 0 var(--space-3);
}

.mode-now.hidden {
  visibility: hidden;
}

.swatches {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
}

.swatch {
  display: grid;
  gap: var(--space-2);
  padding: 0;
  border: 0;
  background: none;
  color: var(--text);
  text-align: left;
  transition: transform var(--dur-fast) var(--ease-out);
}

.swatch:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.swatch-preview {
  position: relative;
  display: grid;
  grid-template-columns: 20px 1fr;
  grid-template-rows: auto auto;
  column-gap: var(--space-2);
  row-gap: var(--space-2);
  align-items: center;
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: var(--bg);
  border: 1px solid var(--border);
  box-shadow:
    0 0 0 2px var(--swatch-gap),
    0 0 0 2px var(--swatch-ring);
  transition: box-shadow var(--dur-base) var(--ease-out);
}

/* The chosen palette's ring opens outward in the sheet's own colours: these
   resolve here, before the preview's .pal-* class redefines --bg and --accent. */
.swatch {
  --swatch-gap: var(--bg);
  --swatch-ring: transparent;
}

.swatch.active {
  --swatch-ring: var(--accent);
}

.swatch.active .swatch-preview {
  box-shadow:
    0 0 0 2px var(--swatch-gap),
    0 0 0 4px var(--swatch-ring);
}

.sp-cover {
  grid-row: span 2;
  width: 20px;
  height: 30px;
  border-radius: 2px;
  background: var(--surface-3);
}

.sp-text {
  display: grid;
  gap: 5px;
}

.sp-title {
  height: 6px;
  border-radius: 2px;
  background: var(--text);
}

.sp-line {
  width: 70%;
  height: 4px;
  border-radius: 2px;
  background: var(--text-muted);
  opacity: 0.55;
}

.sp-accent {
  width: 32px;
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--accent);
}

.swatch-check {
  position: absolute;
  top: var(--space-2);
  right: var(--space-2);
  width: 18px;
  height: 18px;
  padding: 2px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  color: var(--accent-ink);
  animation: check-in var(--dur-base) var(--ease-out);
}

.swatch-name {
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 600;
  line-height: var(--leading-snug);
}

.swatch.active .swatch-name {
  color: var(--accent);
}

@keyframes check-in {
  from {
    opacity: 0;
    transform: scale(0.6);
  }
}
</style>
