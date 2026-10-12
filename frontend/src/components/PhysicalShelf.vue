<script setup lang="ts">
import { TrashIcon } from "@heroicons/vue/24/outline";
import { computed, nextTick, ref } from "vue";
import { useI18n } from "vue-i18n";
import { VueDraggable } from "vue-draggable-plus";
import BookCover from "./BookCover.vue";
import ShelfDecoration from "./ShelfDecoration.vue";
import { defaultSpine, decorationWidth, faceHeight, spineInk, stackSlots, type PhysicalShelfLayout, type ShelfObjectId, type SpineDisplay } from "../physicalShelf";
import type { ShelfItem } from "../types";

const props = defineProps<{
  layout: PhysicalShelfLayout;
  items: ShelfItem[];
  /** Someone else's bookshelf: nothing can be moved, removed or restyled. */
  readonly?: boolean;
  saving?: boolean;
  loading?: boolean;
}>();
const emit = defineEmits<{
  select: [id: ShelfObjectId];
  "update-row": [index: number, row: ShelfObjectId[]];
  "remove-row": [index: number];
  "drag-start": [];
  "drag-end": [];
}>();

type BookView = {
  display: SpineDisplay;
  variant: number;
  lines: 1 | 2;
  author: boolean;
  lean: boolean;
  level?: number;
  style: Record<string, string>;
};

const { t } = useI18n();
const dragging = ref(false);
let suppressClickUntil = 0;
const byId = computed(() => new Map(props.items.map(item => [item.id, item])));
const decorations = computed(() => props.layout.decorations ?? {});
// Placeholder spines while the shelf loads: [width, height] per book, per board.
const skeletonRows = [[[34, 172], [27, 156], [41, 184], [30, 163], [36, 176]], [[29, 168], [38, 152], [32, 180]], [[40, 160], [28, 174], [33, 166], [37, 186]]];

function displayOf(id: number): SpineDisplay {
  return props.layout.spines[String(id)]?.display ?? "spine";
}

/** Lettering is fitted per book so long titles wrap onto a second line instead of being cut off. */
const bookViews = computed(() => {
  const views = new Map<number, BookView>();
  for (const row of props.layout.rows) {
    const slots = stackSlots(row, (id) => {
      const item = byId.value.get(id);
      if (!item || displayOf(id) !== "flat") return null;
      const base = defaultSpine(item);
      return { length: base.height, thickness: props.layout.spines[String(id)]?.width ?? base.width };
    });
    row.forEach((id, index) => {
      const item = typeof id === "number" ? byId.value.get(id) : undefined;
      if (typeof id !== "number" || !item) return;
      const base = defaultSpine(item);
      const custom = props.layout.spines[String(id)];
      const display = displayOf(id);
      const color = custom?.color ?? base.color;
      const width = custom?.width ?? base.width;
      const style: Record<string, string> = {
        "--spine-color": color,
        "--spine-ink": spineInk(color),
        "--spine-width": `${width}px`,
        "--spine-height": `${base.height}px`,
      };
      if (display === "cover") {
        views.set(id, { display, variant: base.variant, lines: 1, author: false, lean: false, style: { "--face-height": `${faceHeight(item)}px` } });
        return;
      }
      if (display === "flat") {
        const slot = slots.get(id)!;
        const letters = Math.max(1, [...item.book.title].length) * (base.variant === 2 ? .68 : .52);
        style["--spine-title-size"] = `${Math.max(7.5, Math.min(12, width - 12, (base.height - 34) / letters)).toFixed(2)}px`;
        style["--stack-lift"] = `${slot.lift}px`;
        style["--stack-span"] = `${slot.span}px`;
        views.set(id, { display, variant: base.variant, lines: 1, author: false, lean: false, level: slot.level, style });
        return;
      }
      const length = Math.max(1, [...item.book.title].length);
      const perChar = base.variant === 2 ? .68 : .52;
      const largest = base.variant === 2 ? 11 : 13;
      const room = base.height - 52;
      const author = width >= 32;
      let size = Math.min(largest, room / (length * perChar), width - (author ? 22 : 10));
      let lines: 1 | 2 = 1;
      if (size < 9.5 && width >= 34) {
        const wrapped = Math.min(largest - 1, room / ((Math.ceil(length / 2) + 4) * perChar), (width - 10) / 2.3);
        if (wrapped > size) { size = wrapped; lines = 2; }
      }
      style["--spine-title-size"] = `${Math.max(7, size).toFixed(2)}px`;
      // A book at the end of a half-empty board leans on its neighbour, as it would on a real shelf.
      const before = row[index - 1];
      const lean = index === row.length - 1 && row.length <= 6 && typeof before === "number" && displayOf(before) === "spine";
      views.set(id, { display, variant: base.variant, lines, author: author && lines === 1, lean, style });
    });
  }
  return views;
});

function objectClass(id: ShelfObjectId) {
  if (typeof id === "string") return "shelf-decoration";
  return bookViews.value.get(id)?.display === "cover" ? "book-face" : "book-spine";
}

function objectLabel(id: ShelfObjectId) {
  if (typeof id === "number") {
    const book = byId.value.get(id)?.book;
    return t("physicalShelf.bookLabel", { title: book?.title, author: book?.authors });
  }
  return t("physicalShelf.decorationLabel", { name: t(`physicalShelf.${decorations.value[id]?.kind}`) });
}

function select(id: ShelfObjectId) {
  if (dragging.value || Date.now() < suppressClickUntil) return;
  emit("select", id);
}

function onDragStart() {
  dragging.value = true;
  emit("drag-start");
}

async function onDragEnd() {
  await nextTick();
  dragging.value = false;
  suppressClickUntil = Date.now() + 300;
  emit("drag-end");
}

function compartment(row: ShelfObjectId[], index: number) {
  if (props.readonly) return {};
  return {
    modelValue: row,
    "onUpdate:modelValue": (value: ShelfObjectId[]) => emit("update-row", index, value),
    group: "physical-books",
    animation: 180,
    disabled: props.saving,
    delay: 220,
    delayOnTouchOnly: true,
    touchStartThreshold: 5,
    fallbackTolerance: 5,
    fallbackOnBody: true,
    scroll: true,
    draggable: ".shelf-object",
    ghostClass: "spine-ghost",
    chosenClass: "spine-chosen",
    dragClass: "spine-dragging",
    onStart: onDragStart,
    onEnd: onDragEnd,
  };
}
</script>

<template>
  <div class="shelf-room">
    <div v-if="loading" class="physical-shelf" data-material="oak" aria-busy="true">
      <span class="visually-hidden">{{ t('physicalShelf.loading') }}</span>
      <div v-for="(row, rowIndex) in skeletonRows" :key="rowIndex" class="physical-shelf-row" aria-hidden="true">
        <div class="shelf-compartment">
          <span v-for="([width, height], index) in row" :key="index" class="shelf-object skeleton skeleton-spine" :style="{ '--spine-width': `${width}px`, '--spine-height': `${height}px` }"></span>
        </div>
      </div>
      <div class="shelf-base" aria-hidden="true"></div>
    </div>
    <div v-else class="physical-shelf" :data-material="layout.material" :data-dragging="dragging || undefined" :data-readonly="readonly || undefined" :aria-label="t('physicalShelf.title')" :aria-busy="saving">
      <div v-if="layout.lights" class="shelf-lights" aria-hidden="true"></div>
      <div v-for="(row, rowIndex) in layout.rows" :key="rowIndex" class="physical-shelf-row" :data-row="rowIndex">
        <button v-if="!readonly" type="button" class="remove-shelf-row" :disabled="saving || layout.rows.length === 1" :aria-label="t('physicalShelf.removeRow', { number: rowIndex + 1 })" :title="t(layout.rows.length === 1 ? 'physicalShelf.lastRow' : 'physicalShelf.removeRowHint')" @click="emit('remove-row', rowIndex)"><TrashIcon aria-hidden="true" /></button>
        <component :is="readonly ? 'div' : VueDraggable" class="shelf-compartment" v-bind="compartment(row, rowIndex)">
          <button v-for="id in row" :key="id" type="button" class="shelf-object" :class="objectClass(id)" :data-id="id" :disabled="saving || (readonly && typeof id === 'string')"
            :data-variant="typeof id === 'number' ? bookViews.get(id)?.variant : undefined" :data-lines="typeof id === 'number' ? bookViews.get(id)?.lines : undefined"
            :data-display="typeof id === 'number' ? bookViews.get(id)?.display : undefined" :data-stack-level="typeof id === 'number' ? bookViews.get(id)?.level : undefined"
            :data-lean="typeof id === 'number' && bookViews.get(id)?.lean ? '' : undefined" :data-kind="typeof id === 'string' ? decorations[id]?.kind : undefined"
            :style="typeof id === 'number' ? bookViews.get(id)?.style : { '--decoration-width': `${decorationWidth(decorations[id].kind)}px` }"
            :aria-label="objectLabel(id)" :title="typeof id === 'number' ? byId.get(id)?.book.title : t(`physicalShelf.${decorations[id].kind}`)"
            @click="select(id)">
            <template v-if="typeof id === 'number' && bookViews.get(id)?.display === 'cover'">
              <BookCover v-if="byId.get(id)" :title="byId.get(id)!.book.title" :cover-id="byId.get(id)!.book.cover_id" :image-url="byId.get(id)!.book.cover_url" :work-key="byId.get(id)!.book.ol_work_key" size="fluid" />
            </template>
            <template v-else-if="typeof id === 'number'">
            <span class="spine-cap" aria-hidden="true"></span>
            <span class="spine-lettering"><span class="spine-title">{{ byId.get(id)?.book.title }}</span><span v-if="bookViews.get(id)?.author" class="spine-author">{{ byId.get(id)?.book.authors }}</span></span>
            <span class="spine-emblem" aria-hidden="true"></span>
            </template>
            <ShelfDecoration v-else :kind="decorations[id].kind" :color="decorations[id].color" />
          </button>
          <span v-if="!row.length" class="empty-compartment">{{ readonly ? '' : t(items.length ? 'physicalShelf.dropHere' : 'physicalShelf.emptyShelf') }}</span>
        </component>
      </div>
      <div class="shelf-base" aria-hidden="true"></div>
    </div>
  </div>
</template>

<style scoped>
/* The room: a pool of warm light on the wall and a soft shadow on the floor. */
.shelf-room { position: relative; padding: 34px 26px 26px; background: radial-gradient(ellipse 75% 60% at 50% 0, color-mix(in srgb, #ffcf8f 13%, transparent), transparent 72%); }
.shelf-room::after { content: ''; position: absolute; left: 6%; right: 6%; bottom: 12px; height: 26px; border-radius: 50%; background: #000; opacity: .28; filter: blur(14px); pointer-events: none; }

.physical-shelf {
  --wood: #c0935f; --wood-light: #dcb685; --wood-hi: #f0d2a6; --wood-dark: #8c6239; --wood-back: #9d774c; --grain: #5a381b; --shelf-label: #3f2a17;
  --spine-scale: 1; --row-h: 212px; --plank: 15px; --frame: 15px;
  position: relative; z-index: 1; padding: 13px var(--frame) 0; border-radius: 3px;
  background:
    repeating-linear-gradient(90.6deg, transparent 0 5px, color-mix(in srgb, var(--grain) 9%, transparent) 5px 6px, transparent 6px 13px, color-mix(in srgb, var(--grain) 5%, transparent) 13px 15px),
    linear-gradient(90deg, var(--wood-light), var(--wood) 15px, var(--wood) calc(100% - 15px), var(--wood-dark));
  box-shadow: 0 22px 40px -18px #000000a6, 0 2px 6px #00000040, inset 0 1px 0 var(--wood-hi), inset 1px 0 0 #ffffff30, inset -1px 0 0 #00000030;
}
/* Top board, overhanging the carcass slightly. */
.physical-shelf::before { content: ''; position: absolute; left: -7px; right: -7px; top: -7px; height: 12px; border-radius: 2px; background: linear-gradient(180deg, var(--wood-hi), var(--wood-light) 30%, var(--wood) 75%, var(--wood-dark)); box-shadow: 0 4px 6px -2px #00000066; }
.physical-shelf[data-material="walnut"] { --wood: #6a4732; --wood-light: #8a6046; --wood-hi: #a87c5e; --wood-dark: #3b261b; --wood-back: #4a3023; --grain: #1c0f08; --shelf-label: #e9d3bb; }
.physical-shelf[data-material="white"] { --wood: #ebe7dd; --wood-light: #f8f5ee; --wood-hi: #ffffff; --wood-dark: #c0b9aa; --wood-back: #cfc9bb; --grain: #7f7663; --shelf-label: #5d5648; }
.physical-shelf[data-material="black"] { --wood: #2c2b2d; --wood-light: #403e40; --wood-hi: #5d5a5b; --wood-dark: #131213; --wood-back: #1d1c1e; --grain: #000000; --shelf-label: #d8d2c8; }

/* String lights draped under the top board: a scalloped wire with a bulb in each dip. */
.shelf-lights {
  --wire: color-mix(in srgb, var(--wood-hi) 38%, #0d0906);
  position: absolute; z-index: 3; left: var(--frame); right: var(--frame); top: 13px; height: 30px; pointer-events: none;
  background:
    radial-gradient(circle at 50% 15px, #fff6d8 0 2.2px, #ffd27a 3.4px, transparent 4.6px) 0 0 / 46px 30px round no-repeat,
    radial-gradient(ellipse 50% 13px at 50% 0, transparent calc(100% - 1.5px), var(--wire) calc(100% - 1.5px), var(--wire) 100%, transparent calc(100% + .8px)) 0 0 / 46px 30px round no-repeat;
}
.shelf-lights::after { content: ''; position: absolute; inset: -4px 0 -64px; background: radial-gradient(circle at 50% 19px, #ffc56666, #ffc56622 16px, transparent 30px) 0 0 / 46px 100% round no-repeat; mix-blend-mode: screen; }
@media (prefers-reduced-motion: no-preference) {
  .shelf-lights::after { animation: shelf-twinkle 3.4s ease-in-out infinite alternate; }
}
@keyframes shelf-twinkle { from { opacity: .72; } to { opacity: 1; } }

.physical-shelf-row { position: relative; box-shadow: inset 12px 0 14px -10px #000000a0, inset -12px 0 14px -10px #000000a0; }
/* Each wrapped line of books is one board: the back wall, the shadows and the plank repeat per line. */
.shelf-compartment {
  --tile: calc(var(--row-h) + var(--plank));
  position: relative; display: flex; flex-wrap: wrap; align-items: flex-end; column-gap: 2px; min-height: var(--tile); padding: 0 22px;
  background:
    linear-gradient(to top, var(--wood-dark) 0, var(--wood) 3px, var(--wood-light) calc(var(--plank) - 2px), var(--wood-hi) calc(var(--plank) - 1px), var(--wood-hi) var(--plank), transparent var(--plank)) 0 0 / 100% var(--tile),
    linear-gradient(to top, transparent var(--plank), #00000052 var(--plank), #0000001c calc(var(--plank) + 7px), transparent calc(var(--plank) + 22px)) 0 0 / 100% var(--tile),
    linear-gradient(to bottom, #00000075 0, #00000038 14px, #00000012 46px, transparent 90px) 0 0 / 100% var(--tile),
    repeating-linear-gradient(90.4deg, transparent 0 9px, color-mix(in srgb, var(--grain) 10%, transparent) 9px 10px, transparent 10px 27px, color-mix(in srgb, var(--grain) 6%, transparent) 27px 29px),
    var(--wood-back);
}
.shelf-base { height: 22px; background: linear-gradient(180deg, #00000040, transparent 5px), linear-gradient(180deg, var(--wood), var(--wood-dark)); box-shadow: inset 0 -1px 0 #00000040; }

.shelf-object { --obj-h: calc(145px * var(--spine-scale)); flex-shrink: 0; height: var(--obj-h); margin: calc(var(--row-h) - var(--obj-h)) 0 var(--plank); }
.skeleton-spine { --obj-h: calc(var(--spine-height) * var(--spine-scale)); width: calc(var(--spine-width) * var(--spine-scale)); border-radius: 2px 2px 0 0; background: color-mix(in srgb, var(--wood-hi) 34%, transparent); }
.book-spine {
  --obj-h: calc(var(--spine-height) * var(--spine-scale));
  position: relative; display: flex; align-items: center; flex-direction: column; width: calc(var(--spine-width) * var(--spine-scale)); border: 0; border-radius: 2px 2px 0 0; padding: calc(20px * var(--spine-scale)) 3px calc(10px * var(--spine-scale)); color: var(--spine-ink);
  background:
    linear-gradient(90deg, #00000057 0, #00000012 9%, #ffffff2b 24%, #ffffff0d 46%, #0000000f 76%, #00000052 100%),
    repeating-linear-gradient(0deg, #ffffff0a 0 1px, transparent 1px 3px),
    var(--spine-color);
  box-shadow: 4px 2px 6px -2px #00000070, inset 0 1px 0 #ffffff38;
  cursor: grab; user-select: none; -webkit-user-select: none; text-align: center;
  transition: transform var(--dur-base) var(--ease), filter var(--dur-base), box-shadow var(--dur-base);
}
/* Foil rules at head and tail. */
.spine-cap, .book-spine::after { content: ''; position: absolute; left: 0; right: 0; height: calc(5px * var(--spine-scale)); border-top: 1px solid currentColor; border-bottom: 1px solid currentColor; opacity: .42; pointer-events: none; }
.spine-cap { top: calc(8px * var(--spine-scale)); }
.book-spine::after { bottom: calc(8px * var(--spine-scale)); }
.book-spine:hover, .book-face:hover { transform: translateY(-9px); filter: brightness(1.08); }
.book-spine:hover { box-shadow: 6px 10px 12px -4px #00000080, inset 0 1px 0 #ffffff38; }
.book-spine:focus-visible, .book-face:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; transform: translateY(-9px); }
.physical-shelf[data-dragging] .shelf-object:hover { transform: none; filter: none; }
.physical-shelf[data-readonly] .shelf-object { cursor: pointer; }
.physical-shelf[data-readonly] .shelf-decoration { cursor: default; }
.book-spine[data-lean] { rotate: -4.5deg; transform-origin: bottom left; margin-left: calc(var(--obj-h) * .082); }
.spine-lettering { display: flex; flex-direction: row-reverse; align-items: stretch; justify-content: center; gap: calc(3px * var(--spine-scale)); flex: 1; min-height: 0; width: 100%; }
.spine-title, .spine-author { writing-mode: vertical-rl; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; text-align: center; line-height: 1.12; }
.spine-title { font-family: var(--serif); font-size: calc(var(--spine-title-size) * var(--spine-scale)); font-weight: 600; letter-spacing: .015em; }
.book-spine[data-lines="2"] .spine-title { white-space: normal; text-wrap: balance; text-overflow: clip; }
.spine-author { font-size: calc(7.5px * var(--spine-scale)); opacity: .78; letter-spacing: .05em; text-align: end; }
.spine-emblem { flex-shrink: 0; margin: calc(6px * var(--spine-scale)) 0 calc(8px * var(--spine-scale)); width: calc(7px * var(--spine-scale)); height: calc(7px * var(--spine-scale)); background: currentColor; opacity: .7; rotate: 45deg; }
/* Variant 1: wide bands and a ringed emblem. */
.book-spine[data-display="spine"][data-variant="1"] .spine-cap, .book-spine[data-display="spine"][data-variant="1"]::after { height: calc(9px * var(--spine-scale)); border-width: 2px 0 1px; opacity: .5; }
.book-spine[data-variant="1"] .spine-emblem { border-radius: 50%; background: none; border: 1.5px solid currentColor; rotate: none; }
/* Variant 2: modern paperback, sans capitals on a solid head band. */
.book-spine[data-variant="2"] .spine-title { font-family: var(--sans); font-weight: 700; text-transform: uppercase; letter-spacing: .07em; }
.book-spine[data-display="spine"][data-variant="2"] .spine-cap { top: 0; height: calc(11px * var(--spine-scale)); border: 0; background: currentColor; opacity: .3; }
.book-spine[data-display="spine"][data-variant="2"]::after { border-top: 0; height: 0; bottom: calc(7px * var(--spine-scale)); }
.book-spine[data-variant="2"] .spine-emblem { width: calc(11px * var(--spine-scale)); height: calc(3px * var(--spine-scale)); rotate: none; }
/* Variant 3: a paper title label. */
.book-spine[data-display="spine"][data-variant="3"] .spine-lettering { padding: calc(6px * var(--spine-scale)) 0; border-block: 1px solid color-mix(in srgb, currentColor 45%, transparent); background: color-mix(in srgb, currentColor 11%, transparent); }
.book-spine[data-display="spine"][data-variant="3"] .spine-cap, .book-spine[data-display="spine"][data-variant="3"]::after { display: none; }
.book-spine[data-display="spine"][data-variant="3"] { padding-top: calc(14px * var(--spine-scale)); }

/* Lying books: the spine turned on its side. Neighbours share one footprint and pile up. */
.book-spine[data-display="flat"] {
  --obj-h: calc(var(--spine-width) * var(--spine-scale));
  --lift: calc(var(--stack-lift, 0px) * var(--spine-scale));
  flex-direction: row; width: calc(var(--spine-height) * var(--spine-scale)); padding: 0 calc(17px * var(--spine-scale)); border-radius: 2px;
  margin-top: calc(var(--row-h) - var(--obj-h) - var(--lift));
  margin-bottom: calc(var(--plank) + var(--lift));
  margin-right: calc((var(--stack-span) - var(--spine-height)) * var(--spine-scale));
  background:
    linear-gradient(180deg, #ffffff30 0, #ffffff0d 28%, #0000000f 70%, #00000057 100%),
    repeating-linear-gradient(90deg, #ffffff0a 0 1px, transparent 1px 3px),
    var(--spine-color);
  box-shadow: 3px 3px 5px -2px #00000070, inset 1px 0 0 #ffffff30;
}
.book-spine[data-display="flat"]:not([data-stack-level="0"]) { margin-left: calc(-1 * var(--stack-span) * var(--spine-scale) - 2px); }
.book-spine[data-display="flat"] .spine-cap, .book-spine[data-display="flat"]::after { top: 0; bottom: 0; left: auto; right: auto; width: calc(5px * var(--spine-scale)); height: auto; border-width: 0 1px; border-style: solid; }
.book-spine[data-display="flat"] .spine-cap { left: calc(8px * var(--spine-scale)); }
.book-spine[data-display="flat"]::after { right: calc(8px * var(--spine-scale)); }
.book-spine[data-display="flat"] .spine-lettering { align-items: center; }
.book-spine[data-display="flat"] .spine-title { writing-mode: horizontal-tb; width: 100%; }
.book-spine[data-display="flat"] .spine-emblem { display: none; }

/* Facing out: the cover itself, leaning against the back wall. */
.book-face { --obj-h: calc(var(--face-height) * var(--spine-scale)); width: calc(var(--obj-h) * 2 / 3); margin-inline: 4px; padding: 0; border: 0; background: none; cursor: grab; user-select: none; -webkit-user-select: none; transition: transform var(--dur-base) var(--ease), filter var(--dur-base); }
.book-face :deep(.cover) { height: 100%; border-radius: 2px; box-shadow: 5px 3px 8px -2px #00000080, inset 0 1px 0 #ffffff30; pointer-events: none; }

.spine-ghost { opacity: .25; }
.spine-chosen { cursor: grabbing; }
.spine-dragging { rotate: -5deg; box-shadow: 9px 12px 18px #00000055; }
.empty-compartment { position: absolute; inset: 0 0 var(--plank); display: grid; place-items: center; padding: 0 56px; text-align: center; font-family: var(--serif); font-style: italic; color: var(--shelf-label); opacity: .5; font-size: var(--text-sm); pointer-events: none; }
.remove-shelf-row { position: absolute; z-index: 4; right: 4px; top: 4px; display: grid; place-items: center; width: 40px; height: 40px; border: 0; border-radius: 50%; background: transparent; color: var(--shelf-label); opacity: 0; transition: opacity var(--dur-fast), background var(--dur-fast); }
.physical-shelf-row:hover .remove-shelf-row, .remove-shelf-row:focus-visible { opacity: .6; }
.remove-shelf-row:hover:not(:disabled) { opacity: 1; background: #ffffff30; }
.remove-shelf-row:disabled { cursor: default; }
.physical-shelf-row:hover .remove-shelf-row:disabled { opacity: .2; }
.remove-shelf-row svg { width: 16px; height: 16px; }
@media (hover: none) { .remove-shelf-row { opacity: .4; } }
.shelf-decoration { width: calc(var(--decoration-width) * var(--spine-scale)); display: block; margin-inline: 6px; padding: 0; border: 0; background: none; cursor: grab; user-select: none; transition: transform var(--dur-base) var(--ease); filter: drop-shadow(3px 3px 3px #00000045); }
.shelf-decoration:not(:disabled):hover { transform: translateY(-4px); }
.shelf-decoration:focus-visible { outline-offset: 3px; }
.shelf-decoration :deep(svg) { pointer-events: none; }

@media (max-width: 600px) {
  .shelf-room { padding: 22px 6px 22px; }
  .physical-shelf { padding: 10px var(--frame) 0; --spine-scale: .86; --row-h: 184px; --plank: 12px; --frame: 10px; }
  .shelf-lights { top: 10px; }
  .shelf-compartment { padding: 0 12px; }
  .shelf-base { height: 16px; }
}
</style>
