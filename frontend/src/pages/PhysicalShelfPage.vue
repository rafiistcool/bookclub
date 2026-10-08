<script setup lang="ts">
import { ArrowLeftIcon, ArrowPathIcon, PlusIcon, XMarkIcon, ArrowsRightLeftIcon, CheckIcon } from "@heroicons/vue/24/outline";
import { computed, nextTick, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { VueDraggable } from "vue-draggable-plus";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import BottomSheet from "../components/BottomSheet.vue";
import { statusLabel, starLabel } from "../constants";
import { defaultSpine, moveBook, reconcileLayout, spineInk, type PhysicalShelfLayout, type ShelfMaterial } from "../physicalShelf";
import { useToast } from "../stores/toast";
import type { ShelfItem } from "../types";

const { t } = useI18n();
const toast = useToast();
const items = ref<ShelfItem[]>([]);
const layout = ref<PhysicalShelfLayout>({ rows: [[], [], []], material: "oak", spines: {} });
const loaded = ref(false);
const error = ref("");
const saving = ref(false);
const selectedId = ref<number | null>(null);
const draftColor = ref("");
const draftWidth = ref(40);
const lastUndo = ref<PhysicalShelfLayout | null>(null);
const notice = ref("");
const dragging = ref(false);
let dragSnapshot: PhysicalShelfLayout | null = null;
let suppressClickUntil = 0;
const materials: ShelfMaterial[] = ["oak", "walnut", "white"];
const colors = ["#31584c", "#a04739", "#d7b779", "#33485f", "#ddd2b6", "#77536b", "#363638", "#a77546"];
const byId = computed(() => new Map(items.value.map(item => [item.id, item])));
const selected = computed(() => selectedId.value === null ? null : byId.value.get(selectedId.value) ?? null);
const selectedRow = computed(() => layout.value.rows.findIndex(row => row.includes(selectedId.value ?? -1)));
const selectedIndex = computed(() => layout.value.rows[selectedRow.value]?.indexOf(selectedId.value ?? -1) ?? -1);

function copy(value: PhysicalShelfLayout): PhysicalShelfLayout {
  return JSON.parse(JSON.stringify(value));
}

async function load() {
  loaded.value = false;
  error.value = "";
  try {
    const [shelf, saved] = await Promise.all([api.myShelf(), api.physicalShelfLayout()]);
    items.value = shelf.items;
    layout.value = reconcileLayout(saved, shelf.items);
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("physicalShelf.loadFailed");
  } finally {
    loaded.value = true;
  }
}

async function persist(next: PhysicalShelfLayout, previous = copy(layout.value), remember = true) {
  if (saving.value || error.value) return;
  layout.value = next;
  saving.value = true;
  notice.value = t("physicalShelf.saving");
  try {
    const saved = await api.savePhysicalShelfLayout(copy(next));
    layout.value = reconcileLayout(saved, items.value);
    lastUndo.value = remember ? previous : null;
    notice.value = t("physicalShelf.saved");
  } catch (err) {
    layout.value = previous;
    notice.value = t("physicalShelf.saveFailed");
    toast.show(err instanceof ApiError ? err.message : t("physicalShelf.saveFailed"));
  } finally {
    saving.value = false;
  }
}

function setMaterial(material: ShelfMaterial) {
  if (saving.value || material === layout.value.material) return;
  void persist({ ...copy(layout.value), material });
}

function addRow() {
  if (saving.value || layout.value.rows.length >= 12) return;
  const next = copy(layout.value);
  next.rows.push([]);
  void persist(next);
}

function undo() {
  if (saving.value || !lastUndo.value) return;
  void persist(copy(lastUndo.value), copy(layout.value), false);
}

function beginDrag() {
  dragSnapshot = copy(layout.value);
  dragging.value = true;
}

async function finishDrag() {
  await nextTick();
  dragging.value = false;
  suppressClickUntil = Date.now() + 300;
  const previous = dragSnapshot;
  dragSnapshot = null;
  if (previous && JSON.stringify(previous.rows) !== JSON.stringify(layout.value.rows)) {
    void persist(copy(layout.value), previous);
  }
}

function styleFor(id: number) {
  const item = byId.value.get(id);
  if (!item) return {};
  const base = defaultSpine(item);
  const custom = layout.value.spines[String(id)];
  const color = custom?.color ?? base.color;
  return {
    "--spine-color": color,
    "--spine-ink": spineInk(color),
    "--spine-width": `${custom?.width ?? base.width}px`,
    "--spine-height": `${base.height}px`,
  };
}

function openBook(id: number) {
  if (saving.value || dragging.value || Date.now() < suppressClickUntil) return;
  const item = byId.value.get(id);
  if (!item) return;
  selectedId.value = id;
  const style = layout.value.spines[String(id)] ?? defaultSpine(item);
  draftColor.value = style.color;
  draftWidth.value = style.width;
}

function shiftBook(direction: -1 | 1) {
  if (!selected.value || saving.value) return;
  const index = selectedIndex.value + direction;
  if (index < 0 || index >= layout.value.rows[selectedRow.value].length) return;
  void persist(moveBook(layout.value, selected.value.id, selectedRow.value, index));
}

function moveToRow(event: Event) {
  if (!selected.value || saving.value) return;
  const target = Number((event.target as HTMLSelectElement).value);
  if (target === selectedRow.value) return;
  void persist(moveBook(layout.value, selected.value.id, target, layout.value.rows[target].length));
}

async function saveSpine() {
  if (!selected.value || saving.value || !/^#[0-9a-f]{6}$/i.test(draftColor.value)) return;
  const next = copy(layout.value);
  next.spines[String(selected.value.id)] = { color: draftColor.value, width: Number(draftWidth.value) };
  await persist(next);
  if (notice.value === t("physicalShelf.saved")) selectedId.value = null;
}

onMounted(load);
</script>

<template>
  <section class="physical-shelf-page">
    <RouterLink to="/shelf" class="back-to-shelf"><ArrowLeftIcon aria-hidden="true" />{{ t('physicalShelf.back') }}</RouterLink>
    <header class="physical-shelf-heading">
      <div><p class="eyebrow">{{ t('physicalShelf.eyebrow') }}</p><h1>{{ t('physicalShelf.title') }}</h1><p class="lede">{{ t('physicalShelf.lede') }}</p></div>
      <span v-if="loaded && !error" class="shelf-book-count">{{ t('physicalShelf.bookCount', { count: items.length }) }}</span>
    </header>

    <div v-if="error" class="empty" role="alert"><p class="error">{{ error }}</p><button class="btn btn-ghost" @click="load">{{ t('physicalShelf.retry') }}</button></div>
    <div v-else-if="!loaded" class="shelf-loading" aria-busy="true">{{ t('physicalShelf.loading') }}</div>
    <template v-else>
      <div class="shelf-studio-toolbar">
        <fieldset class="material-picker" :disabled="saving"><legend>{{ t('physicalShelf.material') }}</legend>
          <button v-for="material in materials" :key="material" type="button" class="material-choice" :data-material="material"
            :aria-pressed="layout.material === material" @click="setMaterial(material)"><span class="wood-swatch" aria-hidden="true"></span>{{ t(`physicalShelf.${material}`) }}</button>
        </fieldset>
        <button type="button" class="undo-layout" :disabled="saving || !lastUndo" :aria-label="t('common.undo')" @click="undo"><ArrowPathIcon aria-hidden="true" /><span>{{ t('common.undo') }}</span></button>
      </div>
      <p class="shelf-instruction"><ArrowsRightLeftIcon aria-hidden="true" />{{ t('physicalShelf.dragHint') }}</p>

      <div class="shelf-room">
        <div class="physical-shelf" :data-material="layout.material" :aria-label="t('physicalShelf.title')" :aria-busy="saving">
          <div v-for="(row, rowIndex) in layout.rows" :key="rowIndex" class="physical-shelf-row" :data-row="rowIndex">
            <span class="shelf-row-number" aria-hidden="true">{{ String(rowIndex + 1).padStart(2, '0') }}</span>
            <VueDraggable v-model="layout.rows[rowIndex]" class="shelf-compartment" group="physical-books"
              :animation="180" :disabled="saving" :delay="220" :delay-on-touch-only="true" :touch-start-threshold="5" :fallback-tolerance="5"
              :fallback-on-body="true" :scroll="true" direction="horizontal"
              draggable=".book-spine" ghost-class="spine-ghost" chosen-class="spine-chosen" drag-class="spine-dragging"
              @start="beginDrag" @end="finishDrag">
              <button v-for="id in row" :key="id" type="button" class="book-spine" :data-id="id" :disabled="saving"
                :data-variant="byId.get(id) ? defaultSpine(byId.get(id)!).variant : 0" :style="styleFor(id)"
                :aria-label="t('physicalShelf.bookLabel', { title: byId.get(id)?.book.title, author: byId.get(id)?.book.authors })"
                :title="byId.get(id)?.book.title" @click="openBook(id)">
                <span class="spine-cap" aria-hidden="true"></span>
                <span class="spine-title">{{ byId.get(id)?.book.title }}</span>
                <span class="spine-author">{{ byId.get(id)?.book.authors }}</span>
                <span class="spine-emblem" aria-hidden="true">✦</span>
              </button>
              <span v-if="!row.length" class="empty-compartment">{{ t(items.length ? 'physicalShelf.dropHere' : 'physicalShelf.emptyShelf') }}</span>
            </VueDraggable>
            <div class="shelf-plank" aria-hidden="true"></div>
          </div>
          <div class="shelf-base" aria-hidden="true"></div>
        </div>
        <div class="shelf-feet" aria-hidden="true"><span></span><span></span></div>
      </div>

      <footer class="shelf-studio-footer">
        <button type="button" class="add-shelf-row" :disabled="saving || layout.rows.length >= 12" @click="addRow"><PlusIcon aria-hidden="true" />{{ t('physicalShelf.addRow') }}</button>
        <span class="save-notice" role="status" aria-live="polite"><CheckIcon v-if="notice === t('physicalShelf.saved')" aria-hidden="true" />{{ notice || t('physicalShelf.autoSave') }}</span>
      </footer>
      <p class="spine-explanation">{{ t('physicalShelf.spineHint') }}</p>
      <RouterLink v-if="!items.length" class="btn btn-primary empty-discover" to="/discover">{{ t('home.findBook') }}</RouterLink>
    </template>

    <BottomSheet v-if="selected" :title="t('physicalShelf.editSpine')" @close="!saving && (selectedId = null)">
      <div class="spine-sheet-book">
        <BookCover :title="selected.book.title" :cover-id="selected.book.cover_id" :image-url="selected.book.cover_url" :work-key="selected.book.ol_work_key" size="md" />
        <div><h3>{{ selected.book.title }}</h3><p v-if="selected.status === 'finished' && selected.rating" class="spine-sheet-stars" :aria-label="t('book.starsOf', { n: selected.rating })">{{ starLabel(selected.rating) }}</p><p class="muted">{{ selected.book.authors }}</p><span class="finer subtle">{{ statusLabel(selected.status) }}</span></div>
        <button type="button" class="icon-btn close-spine-sheet" :aria-label="t('physicalShelf.close')" :disabled="saving" @click="selectedId = null"><XMarkIcon aria-hidden="true" /></button>
      </div>
      <form class="spine-editor" @submit.prevent="saveSpine">
        <label for="spine-color">{{ t('physicalShelf.color') }}</label>
        <div class="spine-color-options"><button v-for="color in colors" :key="color" type="button" class="spine-color-swatch" :style="{ background: color }" :aria-label="t('physicalShelf.chooseColor', { color })" :aria-pressed="draftColor === color" :disabled="saving" @click="draftColor = color"></button><input id="spine-color" v-model="draftColor" class="spine-color" type="color" :disabled="saving" /></div>
        <label for="spine-width">{{ t('physicalShelf.width') }}<span>{{ draftWidth }}</span></label>
        <input id="spine-width" v-model.number="draftWidth" class="spine-width" type="range" min="26" max="64" :disabled="saving" />
        <fieldset class="spine-position" :disabled="saving"><legend>{{ t('physicalShelf.position') }}</legend><div class="spine-position-controls">
          <button type="button" class="btn btn-ghost move-book-left" :disabled="saving || selectedIndex === 0" @click="shiftBook(-1)">{{ t('physicalShelf.left') }}</button>
          <select class="move-book-row" :value="selectedRow" :aria-label="t('physicalShelf.chooseRow')" @change="moveToRow"><option v-for="(_, index) in layout.rows" :key="index" :value="index">{{ t('physicalShelf.row', { number: index + 1 }) }}</option></select>
          <button type="button" class="btn btn-ghost move-book-right" :disabled="saving || selectedIndex === layout.rows[selectedRow].length - 1" @click="shiftBook(1)">{{ t('physicalShelf.right') }}</button>
        </div></fieldset>
        <button type="submit" class="btn btn-primary save-spine" :disabled="saving">{{ t('physicalShelf.apply') }}</button>
        <RouterLink class="btn btn-ghost" :to="`/book/${selected.book.ol_work_key.replace('/works/', '')}`">{{ t('physicalShelf.openBook') }}</RouterLink>
      </form>
    </BottomSheet>
  </section>
</template>

<style scoped>
.physical-shelf-page { max-width: 940px; margin: 0 auto; }
.back-to-shelf { display: inline-flex; align-items: center; gap: 8px; color: var(--text-muted); text-decoration: none; font-size: var(--text-sm); margin-bottom: 22px; }
.back-to-shelf svg { width: 16px; height: 16px; }
.physical-shelf-heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 26px; }
.eyebrow { color: var(--accent); text-transform: uppercase; letter-spacing: .14em; font-size: var(--text-2xs); font-weight: 650; margin-bottom: 8px; }
.physical-shelf-heading h1 { font-size: clamp(1.9rem, 4vw, 2.75rem); font-weight: 550; }
.physical-shelf-heading .lede { margin-top: 9px; font-size: var(--text-md); }
.shelf-book-count { font-size: var(--text-sm); white-space: nowrap; color: var(--text-muted); border: 1px solid var(--border); border-radius: var(--radius-pill); padding: 5px 12px; }
.shelf-studio-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-bottom: 16px; border-bottom: 1px solid var(--border); }
.material-picker { display: flex; gap: 6px; padding: 0; margin: 0; border: 0; align-items: center; }
.material-picker legend { float: left; margin-right: 12px; font-size: var(--text-sm); color: var(--text-muted); line-height: 40px; }
.material-choice { display: flex; align-items: center; gap: 7px; border: 1px solid transparent; border-radius: var(--radius-pill); padding: 7px 11px; background: transparent; font-size: var(--text-sm); min-height: 40px; }
.material-choice[aria-pressed="true"] { background: var(--surface); border-color: var(--border-strong); }
.wood-swatch { width: 18px; height: 18px; border-radius: 50%; box-shadow: inset 0 0 0 1px #00000014; background: repeating-linear-gradient(12deg, #b78b59 0px, #c59b6d 3px, #cfaa7d 5px, #c39a6e 8px); }
[data-material="walnut"] .wood-swatch { background: repeating-linear-gradient(12deg, #573c2c 0px, #75523b 3px, #886148 5px, #70513d 8px); }
[data-material="white"] .wood-swatch { background: #eeeae2; }
.undo-layout { display: flex; gap: 6px; align-items: center; border: 0; background: none; color: var(--text-muted); font-size: var(--text-sm); min-height: 44px; }
.undo-layout svg { width: 17px; height: 17px; }
.undo-layout:disabled { opacity: .4; cursor: default; }
.shelf-instruction { display: flex; gap: 8px; align-items: center; color: var(--text-subtle); font-size: var(--text-sm); margin: 16px 0 22px; }
.shelf-instruction svg { width: 16px; height: 16px; flex-shrink: 0; }
.shelf-room { padding: 0 16px; }
.physical-shelf { --wood: #c59a6c; --wood-light: #debb91; --wood-dark: #9c7048; --wood-back: #c9ac87; --grain: #69462916; --shelf-label: #5c4029; --spine-scale: 1; padding: 14px 16px 0; background: repeating-linear-gradient(88deg, transparent 0, var(--grain) 1px, transparent 3px, transparent 16px), linear-gradient(90deg, var(--wood-light), var(--wood) 3%, var(--wood-light) 95%, var(--wood-dark)); border-radius: 5px 5px 2px 2px; box-shadow: 0 12px 26px #38240e22, inset 1px 1px 1px #ffffff66, inset -2px 0 3px #0000001c; }
.physical-shelf[data-material="walnut"] { --wood: #6c4b36; --wood-light: #92674a; --wood-dark: #402b21; --wood-back: #88654c; --grain: #21130925; --shelf-label: #e7cfb5; }
.physical-shelf[data-material="white"] { --wood: #e1ddd3; --wood-light: #f7f4ec; --wood-dark: #bbb5a6; --wood-back: #e6e1d6; --grain: #968d7c0b; --shelf-label: #817869; }
.physical-shelf-row { position: relative; background: linear-gradient(180deg, #00000018, transparent 30%, #ffffff0a 65%, #0000000d), repeating-linear-gradient(92deg, var(--grain) 0, transparent 2px, transparent 23px), var(--wood-back); box-shadow: inset 5px 5px 15px #27140425, inset -4px 0 8px #27140414; }
.shelf-row-number { position: absolute; top: 10px; left: 12px; font-family: var(--mono); font-size: 10px; opacity: .55; color: var(--shelf-label); pointer-events: none; }
.shelf-compartment { position: relative; display: flex; align-items: flex-end; gap: 3px; height: 217px; padding: 20px 24px 0; overflow-x: auto; overflow-y: hidden; scrollbar-width: thin; scrollbar-color: var(--wood-dark) var(--wood-back); }
.shelf-plank { position: relative; height: 18px; z-index: 2; background: repeating-linear-gradient(0deg, transparent 0px, var(--grain) 1px, transparent 2px, transparent 6px), linear-gradient(180deg, var(--wood-light), var(--wood) 30%, var(--wood-dark)); box-shadow: 0 7px 7px #29180922, inset 0 1px #ffffff55; }
.shelf-base { height: 18px; background: linear-gradient(180deg, var(--wood), var(--wood-dark)); box-shadow: inset 0 1px 3px #00000020; }
.shelf-feet { display: flex; justify-content: space-between; padding: 0 27px; }
.shelf-feet span { height: 14px; width: 25px; background: #685344; box-shadow: 0 5px 7px #00000018; border-radius: 0 0 2px 2px; }
.book-spine { position: relative; display: flex; align-items: center; flex-direction: column; flex-shrink: 0; width: calc(var(--spine-width) * var(--spine-scale)); height: calc(var(--spine-height) * var(--spine-scale)); border: 0; border-radius: 3px 2px 0 0; padding: 16px 3px 9px; color: var(--spine-ink); background: linear-gradient(90deg, #00000032 0, #ffffff12 10%, transparent 25%, #ffffff09 75%, #00000026 92%, #00000042), var(--spine-color); box-shadow: 2px 0 3px #00000030, inset 1px 0 0 #ffffff25, inset -2px 0 1px #00000021; cursor: grab; user-select: none; -webkit-user-select: none; transition: transform var(--dur-base) var(--ease), filter var(--dur-base); text-align: center; }
.book-spine::after { content: ''; position: absolute; inset: 0; opacity: .25; background: repeating-linear-gradient(0deg, transparent 0, #ffffff10 1px, transparent 2px, transparent 4px); border-radius: inherit; pointer-events: none; }
.book-spine:hover { transform: translateY(-6px); filter: brightness(1.06); }
.book-spine:focus-visible { outline: 2px solid var(--spine-ink); outline-offset: -3px; transform: translateY(-5px); }
.spine-title { writing-mode: vertical-rl; max-height: calc(100% - 30px); overflow: hidden; text-overflow: ellipsis; font-family: var(--serif); font-size: 12px; line-height: 1.16; font-weight: 600; letter-spacing: .01em; }
.spine-author { writing-mode: vertical-rl; position: absolute; right: 4px; top: 18px; max-height: calc(100% - 44px); overflow: hidden; text-overflow: ellipsis; font-size: 7px; opacity: .8; letter-spacing: .04em; }
.spine-title { margin-right: 5px; }
.spine-emblem { margin-top: auto; font-size: 11px; opacity: .75; }
.spine-cap { position: absolute; left: 3px; right: 3px; top: 6px; height: 4px; border-top: 1px solid currentColor; border-bottom: 1px solid currentColor; opacity: .3; }
.book-spine[data-variant="1"] .spine-cap { height: 6px; top: 6px; opacity: .55; }
.book-spine[data-variant="2"] .spine-title { font-family: var(--sans); font-weight: 650; text-transform: uppercase; font-size: 10px; letter-spacing: .05em; }
.book-spine[data-variant="3"] .spine-cap { left: 8px; right: 8px; }
.spine-ghost { opacity: .25; }
.spine-chosen { cursor: grabbing; }
.spine-dragging { transform: rotate(-5deg); box-shadow: 7px 8px 14px #00000030; }
.empty-compartment { align-self: center; flex: 1; min-width: 150px; text-align: center; padding-top: 16px; font-family: var(--serif); font-style: italic; color: var(--shelf-label); opacity: .65; font-size: var(--text-sm); pointer-events: none; }
.shelf-studio-footer { display: flex; justify-content: space-between; align-items: center; gap: 10px; margin: 24px 16px 0; }
.add-shelf-row { display: flex; align-items: center; gap: 6px; background: none; border: 1px dashed var(--border-strong); border-radius: var(--radius-sm); padding: 9px 12px; font-size: var(--text-sm); }
.add-shelf-row:disabled { opacity: .4; }
.add-shelf-row svg, .save-notice svg { width: 16px; height: 16px; }
.save-notice { display: flex; align-items: center; gap: 5px; color: var(--text-subtle); font-size: var(--text-xs); text-align: right; }
.spine-explanation { text-align: center; font-size: var(--text-xs); color: var(--text-subtle); margin: 22px auto 0; max-width: 560px; }
.empty-discover { display: block; width: fit-content; margin: 20px auto; }
.shelf-loading { min-height: 400px; display: grid; place-items: center; color: var(--text-muted); }
.spine-sheet-book { display: flex; gap: 15px; align-items: center; margin-bottom: 25px; }
.spine-sheet-book > div { flex: 1; }
.spine-sheet-stars { color: var(--accent); font-size: var(--text-sm); }
.close-spine-sheet { align-self: flex-start; }
.close-spine-sheet svg { width: 20px; height: 20px; }
.spine-editor { display: flex; flex-direction: column; gap: 12px; }
.spine-editor > label { display: flex; justify-content: space-between; font-size: var(--text-sm); font-weight: 600; }
.spine-editor > label span { color: var(--text-subtle); font-weight: 400; }
.spine-color-options { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; }
.spine-color-swatch { height: 32px; width: 32px; border-radius: 50%; border: 2px solid var(--bg); box-shadow: 0 0 0 1px var(--border); }
.spine-color-swatch[aria-pressed="true"] { box-shadow: 0 0 0 2px var(--accent); }
.spine-color { height: 34px; width: 38px; padding: 2px; border: 1px solid var(--border); border-radius: 5px; background: var(--surface); }
.spine-width { width: 100%; accent-color: var(--accent); min-height: 30px; }
.spine-position { border: 0; padding: 0; margin: 6px 0 12px; }
.spine-position legend { font-size: var(--text-sm); font-weight: 600; margin-bottom: 8px; }
.spine-position-controls { display: flex; gap: 8px; }
.spine-position-controls select { flex: 1; min-width: 0; }
.spine-position-controls button { padding: 8px 12px; }
@media (max-width: 600px) {
  .back-to-shelf { margin-bottom: 18px; }
  .physical-shelf-heading { align-items: flex-start; margin-bottom: 20px; }
  .physical-shelf-heading .lede { font-size: var(--text-sm); max-width: 260px; }
  .shelf-book-count { padding: 4px 8px; font-size: var(--text-xs); margin-top: 28px; }
  .material-picker { gap: 2px; }
  .material-picker legend { display: none; }
  .material-choice { font-size: var(--text-xs); padding: 7px 8px; }
  .undo-layout span { display: none; }
  .shelf-instruction { font-size: var(--text-xs); line-height: 1.4; margin: 13px 0 19px; }
  .shelf-room { padding: 0; }
  .physical-shelf { padding: 10px 10px 0; --spine-scale: .82; }
  .shelf-compartment { height: 181px; padding: 20px 10px 0; gap: 2px; }
  .shelf-plank { height: 13px; }
  .shelf-base { height: 12px; }
  .spine-title { font-size: 10px; }
  .book-spine[data-variant="2"] .spine-title { font-size: 8px; }
  .spine-author { right: 2px; font-size: 6px; }
  .shelf-row-number { top: 7px; left: 8px; font-size: 8px; }
  .shelf-studio-footer { margin: 20px 0 0; }
  .save-notice { max-width: 145px; }
  .spine-explanation { line-height: 1.5; }
}
</style>
