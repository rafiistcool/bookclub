<script setup lang="ts">
import { ArrowLeftIcon, ArrowUturnLeftIcon, CheckIcon, LightBulbIcon, PaintBrushIcon, PlusIcon, SparklesIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import { computed, nextTick, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { tp } from "../i18n";
import BottomSheet from "../components/BottomSheet.vue";
import PhysicalShelf from "../components/PhysicalShelf.vue";
import ShelfBookInfo from "../components/ShelfBookInfo.vue";
import ShelfDecoration from "../components/ShelfDecoration.vue";
import ShelfViewSwitch from "../components/ShelfViewSwitch.vue";
import { defaultSpine, moveBook, reconcileLayout, removeShelfRow, spineInk, type DecorationKind, type PhysicalShelfLayout, type ShelfMaterial, type ShelfObjectId, type SpineDisplay, type SpineStyle } from "../physicalShelf";
import { useToast } from "../stores/toast";
import type { ShelfItem } from "../types";

const { t } = useI18n();
const toast = useToast();
const items = ref<ShelfItem[]>([]);
const layout = ref<PhysicalShelfLayout>({ rows: [[], [], []], material: "oak", spines: {} });
const loaded = ref(false);
const error = ref("");
const saving = ref(false);
const selectedId = ref<ShelfObjectId | null>(null);
// Tapping a book shows it first; restyling is a second step.
const editing = ref(false);
const draftColor = ref("");
const draftWidth = ref(40);
const draftDisplay = ref<SpineDisplay>("spine");
// Unless picked by hand, colour and thickness keep following the book.
const draftAutoColor = ref(true);
const draftAutoWidth = ref(true);
const lastUndo = ref<PhysicalShelfLayout | null>(null);
const notice = ref("");
const showDecorationPicker = ref(false);
const decorationRow = ref(0);
let dragSnapshot: PhysicalShelfLayout | null = null;
const materials: ShelfMaterial[] = ["oak", "walnut", "white", "black"];
const displays: SpineDisplay[] = ["spine", "flat", "cover"];
const decorationKinds: DecorationKind[] = ["plant", "vase", "candle", "frame", "bookend", "globe"];
const decorationColors: Record<DecorationKind, string> = { plant: "#b57b5c", vase: "#a77546", candle: "#ddd2b6", frame: "#77533e", bookend: "#a77546", globe: "#33485f" };
const colors = ["#31584c", "#a04739", "#d7b779", "#33485f", "#ddd2b6", "#77536b", "#363638", "#a77546"];
const byId = computed(() => new Map(items.value.map(item => [item.id, item])));
const selected = computed(() => typeof selectedId.value === "number" ? byId.value.get(selectedId.value) ?? null : null);
const decorations = computed(() => layout.value.decorations ?? {});
const selectedDecoration = computed(() => typeof selectedId.value === "string" ? decorations.value[selectedId.value] ?? null : null);
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
    void sampleCoverColors();
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("physicalShelf.loadFailed");
  } finally {
    loaded.value = true;
  }
}

/** Ask the server to read spine colours off the covers that have none yet, a batch at a time. */
async function sampleCoverColors() {
  for (let round = 0; round < 5; round++) {
    if (!items.value.some(item => item.book.spine_color == null && item.book.cover_url)) return;
    try {
      const result = await api.spineColors();
      for (const item of items.value) {
        const color = result.colors[String(item.book.id)];
        if (color) item.book.spine_color = color;
      }
      if (!result.remaining) return;
    } catch {
      return; // Purely cosmetic: the palette colours stay.
    }
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

function toggleLights() {
  if (saving.value) return;
  void persist({ ...copy(layout.value), lights: !layout.value.lights });
}

function addRow() {
  if (saving.value || layout.value.rows.length >= 12) return;
  const next = copy(layout.value);
  next.rows.push([]);
  void persist(next);
}

function removeRow(index: number) {
  if (saving.value || layout.value.rows.length === 1) return;
  void persist(removeShelfRow(layout.value, index));
}

function addDecoration(kind: DecorationKind) {
  if (saving.value || Object.keys(decorations.value).length >= 100) return;
  const next = copy(layout.value);
  // randomUUID needs a secure context; a club served over plain http on a LAN has none.
  const uid = typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, "0")).join("");
  const id = `decor:${uid}`;
  next.decorations = { ...next.decorations, [id]: { kind, color: decorationColors[kind] } };
  const row = Math.max(0, Math.min(decorationRow.value, next.rows.length - 1));
  next.rows[row].push(id);
  showDecorationPicker.value = false;
  void persist(next);
}

function openDecoration(id: string) {
  if (saving.value || !decorations.value[id]) return;
  selectedId.value = id;
  draftColor.value = decorations.value[id].color;
}

async function saveDecoration() {
  if (saving.value || typeof selectedId.value !== "string" || !selectedDecoration.value) return;
  const next = copy(layout.value);
  next.decorations = { ...next.decorations, [selectedId.value]: { ...selectedDecoration.value, color: draftColor.value } };
  await persist(next);
  if (notice.value === t("physicalShelf.saved")) selectedId.value = null;
}

async function removeDecoration() {
  if (saving.value || typeof selectedId.value !== "string") return;
  const next = copy(layout.value);
  const id = selectedId.value;
  next.rows = next.rows.map(row => row.filter(objectId => objectId !== id));
  if (next.decorations) delete next.decorations[id];
  await persist(next);
  if (notice.value === t("physicalShelf.saved")) selectedId.value = null;
}

function undo() {
  if (saving.value || !lastUndo.value) return;
  void persist(copy(lastUndo.value), copy(layout.value), false);
}

function beginDrag() {
  dragSnapshot = copy(layout.value);
}

async function finishDrag() {
  await nextTick();
  const previous = dragSnapshot;
  dragSnapshot = null;
  if (previous && JSON.stringify(previous.rows) !== JSON.stringify(layout.value.rows)) {
    void persist(copy(layout.value), previous);
  }
}

function openBook(id: number) {
  if (saving.value || !byId.value.has(id)) return;
  selectedId.value = id;
  editing.value = false;
}

function select(id: ShelfObjectId) {
  if (typeof id === "number") openBook(id);
  else openDecoration(id);
}

function startEditing() {
  const item = selected.value;
  if (!item) return;
  const base = defaultSpine(item);
  const custom = layout.value.spines[String(item.id)];
  draftAutoColor.value = custom?.color == null;
  draftAutoWidth.value = custom?.width == null;
  draftColor.value = custom?.color ?? base.color;
  draftWidth.value = custom?.width ?? base.width;
  draftDisplay.value = custom?.display ?? "spine";
  editing.value = true;
}

function pickColor(color: string) {
  draftColor.value = color;
  draftAutoColor.value = false;
}

function resetColor() {
  if (!selected.value) return;
  draftColor.value = defaultSpine(selected.value).color;
  draftAutoColor.value = true;
}

function resetWidth() {
  if (!selected.value) return;
  draftWidth.value = defaultSpine(selected.value).width;
  draftAutoWidth.value = true;
}

function shiftBook(direction: -1 | 1) {
  if (selectedId.value === null || saving.value) return;
  const index = selectedIndex.value + direction;
  if (index < 0 || index >= layout.value.rows[selectedRow.value].length) return;
  void persist(moveBook(layout.value, selectedId.value, selectedRow.value, index));
}

function moveToRow(event: Event) {
  if (selectedId.value === null || saving.value) return;
  const target = Number((event.target as HTMLSelectElement).value);
  if (target === selectedRow.value) return;
  void persist(moveBook(layout.value, selectedId.value, target, layout.value.rows[target].length));
}

async function saveSpine() {
  if (!selected.value || saving.value || !/^#[0-9a-f]{6}$/i.test(draftColor.value)) return;
  const next = copy(layout.value);
  const style: SpineStyle = {};
  if (!draftAutoColor.value) style.color = draftColor.value;
  if (!draftAutoWidth.value) style.width = Number(draftWidth.value);
  if (draftDisplay.value !== "spine") style.display = draftDisplay.value;
  if (Object.keys(style).length) next.spines[String(selected.value.id)] = style;
  else delete next.spines[String(selected.value.id)];
  await persist(next);
  if (notice.value === t("physicalShelf.saved")) selectedId.value = null;
}

onMounted(load);
</script>

<template>
  <section class="physical-shelf-page">
    <header class="physical-shelf-heading">
      <div>
        <h1>{{ t('physicalShelf.title') }}</h1>
        <p v-if="loaded && !error" class="shelf-meta">
          <span class="shelf-book-count">{{ tp('shelf.books', items.length) }}</span>
          <span class="save-notice" role="status" aria-live="polite"><CheckIcon v-if="notice === t('physicalShelf.saved')" aria-hidden="true" />{{ notice || t('physicalShelf.autoSave') }}</span>
        </p>
      </div>
      <ShelfViewSwitch active="physical" />
    </header>

    <div v-if="error" class="empty" role="alert"><p class="error">{{ error }}</p><button class="btn btn-ghost" @click="load">{{ t('physicalShelf.retry') }}</button></div>
    <PhysicalShelf v-else-if="!loaded" :layout="layout" :items="[]" loading />
    <template v-else>
      <div class="shelf-studio-toolbar">
        <fieldset class="material-picker" :disabled="saving"><legend>{{ t('physicalShelf.material') }}</legend>
          <button v-for="material in materials" :key="material" type="button" class="material-choice" :data-material="material" :title="t(`physicalShelf.${material}`)"
            :aria-pressed="layout.material === material" @click="setMaterial(material)"><span class="wood-swatch" aria-hidden="true"></span><span class="material-name">{{ t(`physicalShelf.${material}`) }}</span></button>
        </fieldset>
        <div class="shelf-tools">
          <button type="button" class="shelf-tool add-decoration" :disabled="saving || Object.keys(decorations).length >= 100" :aria-label="t('physicalShelf.addDecoration')" @click="showDecorationPicker = true"><SparklesIcon aria-hidden="true" /><span>{{ t('physicalShelf.addDecorationShort') }}</span></button>
          <button type="button" class="shelf-tool add-shelf-row" :disabled="saving || layout.rows.length >= 12" :aria-label="t('physicalShelf.addRow')" @click="addRow"><PlusIcon aria-hidden="true" /><span>{{ t('physicalShelf.addRowShort') }}</span></button>
          <button type="button" class="shelf-tool icon-tool toggle-lights" :disabled="saving" :aria-pressed="Boolean(layout.lights)" :aria-label="t('physicalShelf.lights')" :title="t('physicalShelf.lights')" @click="toggleLights"><LightBulbIcon aria-hidden="true" /></button>
          <button type="button" class="shelf-tool icon-tool undo-layout" :disabled="saving || !lastUndo" :aria-label="t('common.undo')" :title="t('common.undo')" @click="undo"><ArrowUturnLeftIcon aria-hidden="true" /></button>
        </div>
      </div>

      <PhysicalShelf :layout="layout" :items="items" :saving="saving" @select="select" @update-row="(index, row) => layout.rows[index] = row"
        @remove-row="removeRow" @drag-start="beginDrag" @drag-end="finishDrag" />

      <p class="shelf-instruction">{{ t('physicalShelf.dragHint') }}</p>
      <RouterLink v-if="!items.length" class="btn btn-primary empty-discover" to="/discover">{{ t('home.findBook') }}</RouterLink>
    </template>

    <BottomSheet v-if="showDecorationPicker" :title="t('physicalShelf.decorate')" @close="showDecorationPicker = false">
      <p class="decoration-picker-hint muted">{{ t('physicalShelf.decorationHint') }}</p>
      <label class="decoration-row-label" for="decoration-row">{{ t('physicalShelf.position') }}</label>
      <select id="decoration-row" v-model.number="decorationRow" class="decoration-row" :disabled="saving"><option v-for="(_, index) in layout.rows" :key="index" :value="index">{{ t('physicalShelf.row', { number: index + 1 }) }}</option></select>
      <div class="decoration-picker"><button v-for="kind in decorationKinds" :key="kind" type="button" class="decoration-choice" :data-kind="kind" :disabled="saving" @click="addDecoration(kind)"><span class="decoration-preview"><ShelfDecoration :kind="kind" :color="decorationColors[kind]" /></span><strong>{{ t(`physicalShelf.${kind}`) }}</strong><span class="finer subtle">{{ t(`physicalShelf.${kind}Hint`) }}</span></button></div>
      <button type="button" class="btn btn-ghost btn-block" @click="showDecorationPicker = false">{{ t('physicalShelf.close') }}</button>
    </BottomSheet>

    <BottomSheet v-if="selectedDecoration" :title="t(`physicalShelf.${selectedDecoration.kind}`)" @close="!saving && (selectedId = null)">
      <div class="decoration-editor-preview"><ShelfDecoration :kind="selectedDecoration.kind" :color="draftColor" /></div>
      <form class="spine-editor decoration-editor" @submit.prevent="saveDecoration">
        <label for="decoration-color">{{ t('physicalShelf.decorationColor') }}</label>
        <div class="spine-color-options"><button v-for="color in colors" :key="color" type="button" class="spine-color-swatch" :style="{ background: color }" :aria-label="t('physicalShelf.chooseColor', { color })" :aria-pressed="draftColor === color" :disabled="saving" @click="draftColor = color"></button><input id="decoration-color" v-model="draftColor" class="decoration-color spine-color" type="color" :disabled="saving" /></div>
        <fieldset class="spine-position" :disabled="saving"><legend>{{ t('physicalShelf.position') }}</legend><div class="spine-position-controls">
          <button type="button" class="btn btn-ghost move-book-left" :disabled="saving || selectedIndex === 0" @click="shiftBook(-1)">{{ t('physicalShelf.left') }}</button>
          <select class="move-book-row" :value="selectedRow" :aria-label="t('physicalShelf.chooseRow')" @change="moveToRow"><option v-for="(_, index) in layout.rows" :key="index" :value="index">{{ t('physicalShelf.row', { number: index + 1 }) }}</option></select>
          <button type="button" class="btn btn-ghost move-book-right" :disabled="saving || selectedIndex === layout.rows[selectedRow].length - 1" @click="shiftBook(1)">{{ t('physicalShelf.right') }}</button>
        </div></fieldset>
        <button type="submit" class="btn btn-primary save-decoration" :disabled="saving">{{ t('physicalShelf.saveDecoration') }}</button>
        <button type="button" class="btn btn-ghost remove-decoration" :disabled="saving" @click="removeDecoration">{{ t('physicalShelf.removeDecoration') }}</button>
        <button type="button" class="btn btn-ghost" :disabled="saving" @click="selectedId = null">{{ t('physicalShelf.close') }}</button>
      </form>
    </BottomSheet>

    <BottomSheet v-if="selected" :title="t(editing ? 'physicalShelf.editSpine' : 'physicalShelf.bookSheet')" @close="!saving && (selectedId = null)">
      <ShelfBookInfo :item="selected">
        <button type="button" class="icon-btn close-spine-sheet" :aria-label="t('physicalShelf.close')" :disabled="saving" @click="selectedId = null"><XMarkIcon aria-hidden="true" /></button>
      </ShelfBookInfo>
      <div v-if="!editing" class="book-sheet-actions">
        <RouterLink class="btn btn-primary open-book" :to="`/book/${selected.book.ol_work_key.replace('/works/', '')}`">{{ t('physicalShelf.openBook') }}</RouterLink>
        <button type="button" class="btn btn-ghost edit-spine" :disabled="saving" @click="startEditing"><PaintBrushIcon aria-hidden="true" />{{ t('physicalShelf.restyle') }}</button>
      </div>
      <form v-else class="spine-editor" @submit.prevent="saveSpine">
        <fieldset class="spine-display" :disabled="saving"><legend>{{ t('physicalShelf.display') }}</legend>
          <div class="segmented"><button v-for="display in displays" :key="display" type="button" :data-display="display" :aria-pressed="draftDisplay === display" @click="draftDisplay = display">{{ t(`physicalShelf.display_${display}`) }}</button></div>
        </fieldset>
        <template v-if="draftDisplay !== 'cover'">
        <label for="spine-color">{{ t('physicalShelf.color') }}<button v-if="!draftAutoColor" type="button" class="text-btn reset-spine-color" @click="resetColor">{{ t('physicalShelf.automatic') }}</button></label>
        <div class="spine-color-row">
          <span class="spine-preview" aria-hidden="true" :style="{ '--spine-color': draftColor, '--spine-ink': spineInk(draftColor), '--spine-width': `${draftWidth}px` }"></span>
          <div class="spine-color-options"><button v-for="color in colors" :key="color" type="button" class="spine-color-swatch" :style="{ background: color }" :aria-label="t('physicalShelf.chooseColor', { color })" :aria-pressed="!draftAutoColor && draftColor === color" :disabled="saving" @click="pickColor(color)"></button><input id="spine-color" v-model="draftColor" class="spine-color" type="color" :disabled="saving" @input="draftAutoColor = false" /></div>
        </div>
        <label for="spine-width">{{ t('physicalShelf.width') }}<button v-if="!draftAutoWidth" type="button" class="text-btn reset-spine-width" @click="resetWidth">{{ t('physicalShelf.automatic') }}</button></label>
        <input id="spine-width" v-model.number="draftWidth" class="spine-width" type="range" min="26" max="64" :disabled="saving" @input="draftAutoWidth = false" />
        </template>
        <fieldset class="spine-position" :disabled="saving"><legend>{{ t('physicalShelf.position') }}</legend><div class="spine-position-controls">
          <button type="button" class="btn btn-ghost move-book-left" :disabled="saving || selectedIndex === 0" @click="shiftBook(-1)">{{ t('physicalShelf.left') }}</button>
          <select class="move-book-row" :value="selectedRow" :aria-label="t('physicalShelf.chooseRow')" @change="moveToRow"><option v-for="(_, index) in layout.rows" :key="index" :value="index">{{ t('physicalShelf.row', { number: index + 1 }) }}</option></select>
          <button type="button" class="btn btn-ghost move-book-right" :disabled="saving || selectedIndex === layout.rows[selectedRow].length - 1" @click="shiftBook(1)">{{ t('physicalShelf.right') }}</button>
        </div></fieldset>
        <button type="submit" class="btn btn-primary save-spine" :disabled="saving">{{ t('physicalShelf.apply') }}</button>
        <button type="button" class="btn btn-ghost back-to-book" :disabled="saving" @click="editing = false"><ArrowLeftIcon aria-hidden="true" />{{ t('physicalShelf.backToBook') }}</button>
      </form>
    </BottomSheet>
  </section>
</template>

<style scoped>
.physical-shelf-page { max-width: 940px; margin: 0 auto; }
.physical-shelf-heading { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; margin-bottom: 20px; }
.physical-shelf-heading h1 { font-size: clamp(1.6rem, 3.4vw, 2.1rem); font-weight: 550; }
.shelf-meta { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; margin-top: 6px; font-size: var(--text-sm); color: var(--text-muted); }
.save-notice { display: inline-flex; align-items: center; gap: 4px; color: var(--text-subtle); }
.save-notice::before { content: '·'; margin-right: 6px; }
.save-notice svg { width: 14px; height: 14px; color: var(--accent); }

.shelf-studio-toolbar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px 12px; margin-bottom: 8px; }
.material-picker { display: flex; gap: 4px; padding: 3px; margin: 0; border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface-2); min-width: 0; }
.material-picker legend { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
.material-choice { display: flex; align-items: center; gap: 7px; border: 0; border-radius: var(--radius-pill); padding: 6px 12px 6px 7px; background: transparent; color: var(--text-muted); font-size: var(--text-sm); font-weight: 600; min-height: 36px; }
.material-choice:hover { color: var(--text); }
.material-choice[aria-pressed="true"] { background: var(--surface); color: var(--text); box-shadow: var(--shadow-1); }
.wood-swatch { width: 22px; height: 22px; border-radius: 50%; box-shadow: inset 0 0 0 1px #00000026, inset 0 2px 2px #ffffff40; background: repeating-linear-gradient(12deg, #b78b59 0px, #c59b6d 3px, #cfaa7d 5px, #c39a6e 8px); }
[data-material="walnut"] .wood-swatch { background: repeating-linear-gradient(12deg, #573c2c 0px, #75523b 3px, #886148 5px, #70513d 8px); }
[data-material="white"] .wood-swatch { background: linear-gradient(140deg, #fbf9f3, #ddd8cc); }
[data-material="black"] .wood-swatch { background: linear-gradient(140deg, #4a4849, #19181a); box-shadow: inset 0 0 0 1px #ffffff30; }
.shelf-tools { display: flex; gap: 6px; margin-left: auto; }
.shelf-tool { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 40px; min-width: 40px; padding: 0 12px; border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface); color: var(--text); font-size: var(--text-sm); font-weight: 600; transition: border-color var(--dur-fast), background var(--dur-fast); }
.shelf-tool:hover:not(:disabled) { border-color: var(--accent-line); background: var(--accent-soft); }
.shelf-tool:disabled { opacity: .4; cursor: default; }
.shelf-tool svg { width: 17px; height: 17px; flex-shrink: 0; }
.icon-tool { padding: 0; }
.toggle-lights[aria-pressed="true"] { border-color: #e2b65a; background: color-mix(in srgb, #ffcf6b 22%, var(--surface)); color: #b07a12; }

.shelf-instruction { text-align: center; color: var(--text-subtle); font-size: var(--text-xs); margin: 4px auto 0; max-width: 520px; line-height: 1.5; }
.empty-discover { display: block; width: fit-content; margin: 20px auto; }

.close-spine-sheet { align-self: flex-start; }
.close-spine-sheet svg { width: 20px; height: 20px; }
.book-sheet-actions { display: flex; flex-direction: column; gap: 10px; margin-top: 22px; }
.book-sheet-actions .btn, .back-to-book { display: inline-flex; align-items: center; justify-content: center; gap: 8px; }
.book-sheet-actions svg, .back-to-book svg { width: 17px; height: 17px; }
.spine-editor { display: flex; flex-direction: column; gap: 12px; margin-top: 22px; }
.decoration-editor { margin-top: 0; }
.spine-editor > label { display: flex; justify-content: space-between; align-items: baseline; font-size: var(--text-sm); font-weight: 600; }
.spine-editor > label span { color: var(--text-subtle); font-weight: 400; }
.spine-display, .spine-position { border: 0; padding: 0; margin: 0; min-width: 0; }
.spine-display legend, .spine-position legend { font-size: var(--text-sm); font-weight: 600; margin-bottom: 8px; }
.spine-display .segmented { display: flex; width: 100%; }
.spine-display .segmented button { flex: 1; }
.spine-color-row { display: flex; align-items: center; gap: 14px; }
.spine-preview { flex-shrink: 0; width: var(--spine-width); height: 84px; border-radius: 2px; color: var(--spine-ink); background: linear-gradient(90deg, #00000057 0, #00000012 9%, #ffffff2b 24%, #ffffff0d 46%, #0000000f 76%, #00000052 100%), var(--spine-color); box-shadow: 3px 2px 6px -2px #00000070, inset 0 7px 0 -6px currentColor, inset 0 -7px 0 -6px currentColor; transition: width var(--dur-fast); }
.spine-color-options { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.spine-color-swatch { height: 34px; width: 34px; border-radius: 50%; border: 2px solid var(--surface); box-shadow: 0 0 0 1px var(--border-strong); }
.spine-color-swatch[aria-pressed="true"] { box-shadow: 0 0 0 2px var(--accent); }
.spine-color { height: 34px; width: 34px; padding: 0; border: 0; border-radius: 50%; overflow: hidden; background: conic-gradient(#e0554a, #e0b84a, #5fae6a, #4a8fe0, #9a5fd0, #e0554a); cursor: pointer; }
.spine-color::-webkit-color-swatch-wrapper { padding: 7px; }
.spine-color::-webkit-color-swatch { border: 2px solid var(--surface); border-radius: 50%; }
.spine-color::-moz-color-swatch { border: 2px solid var(--surface); border-radius: 50%; }
.spine-width { width: 100%; accent-color: var(--accent); min-height: 30px; }
.spine-position { margin-bottom: 6px; }
.spine-position-controls { display: flex; gap: 8px; }
.spine-position-controls select { flex: 1; min-width: 0; }
.spine-position-controls button { padding: 8px 12px; }
.decoration-picker-hint { font-size: var(--text-sm); margin-bottom: 18px; }
.decoration-row-label { display: block; font-size: var(--text-sm); font-weight: 600; margin-bottom: 6px; }
.decoration-row { width: 100%; margin-bottom: 18px; }
.decoration-picker { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-bottom: 18px; }
.decoration-choice { display: flex; flex-direction: column; align-items: center; gap: 5px; padding: 14px 8px 16px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-md); }
.decoration-choice:hover { border-color: var(--accent-line); background: var(--accent-soft); }
.decoration-preview { width: 76px; height: 100px; display: block; margin-bottom: 7px; }
.decoration-choice strong { font-family: var(--serif); font-size: var(--text-md); }
.decoration-editor-preview { height: 145px; width: 95px; margin: 0 auto 20px; }
@media (max-width: 600px) {
  .physical-shelf-heading { flex-direction: column; align-items: flex-start; gap: 14px; margin-bottom: 16px; }
  .material-choice { padding: 6px; }
  .material-name { display: none; }
  .material-choice[aria-pressed="true"] .wood-swatch { outline: 2px solid var(--accent); outline-offset: 2px; }
  .shelf-tool { padding: 0 11px; }
  .icon-tool { padding: 0; }
  .decoration-picker { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
}
</style>
