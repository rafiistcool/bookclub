import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { VueDraggable, type DraggableEvent, type UseDraggableReturn } from "vue-draggable-plus";
import { statusLabel } from "../constants";
import type { DecorationKind, ShelfDecoration, ShelfObjectId } from "../physicalShelf";
import type { ShelfItem } from "../types";

const myShelf = vi.fn();
const physicalShelfLayout = vi.fn();
const savePhysicalShelfLayout = vi.fn();
const patchShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    myShelf: (...args: unknown[]) => myShelf(...args),
    physicalShelfLayout: (...args: unknown[]) => physicalShelfLayout(...args),
    savePhysicalShelfLayout: (...args: unknown[]) => savePhysicalShelfLayout(...args),
    patchShelf: (...args: unknown[]) => patchShelf(...args),
  },
}));

import PhysicalShelfPage from "./PhysicalShelfPage.vue";

type Layout = {
  rows: ShelfObjectId[][];
  material: "oak" | "walnut" | "white" | "black";
  lights?: boolean;
  spines: Record<string, { color?: string; width?: number; display?: string }>;
  decorations?: Record<string, ShelfDecoration>;
};

const draggable = {
  name: "VueDraggable",
  props: ["modelValue", "disabled", "delay", "delayOnTouchOnly", "group"],
  emits: ["update:modelValue", "start", "end"],
  template: '<div><slot /></div>',
};

function item(id: number, title: string, status: ShelfItem["status"] = "want_to_read"): ShelfItem {
  return {
    id, status, position: id - 1, shelf_position: id - 1,
    updated_at: "2026-10-08T10:00:00Z", rating: status === "finished" ? 5 : null,
    take: "", dnf_reason: "", progress: null,
    book: {
      id: id + 100, title, authors: "An Author", cover_id: null,
      cover_url: null, year: 2020, ol_work_key: `/works/OL${id}W`,
    },
  };
}

const books = [item(1, "First"), item(2, "Finished", "finished"), item(3, "Reading", "currently_reading")];
const saved: Layout = { rows: [[2, 1], [3], []], material: "walnut", spines: { "2": { color: "#123456", width: 40 } } };
const decorated: Layout = {
  ...saved,
  rows: [[2, "decor:plant-1", 1], [3, "decor:frame-1"], []],
  decorations: {
    "decor:plant-1": { kind: "plant", color: "#b57b5c" },
    "decor:frame-1": { kind: "frame", color: "#77533e" },
  },
};

function rowBooks(wrapper: VueWrapper): string[][] {
  return wrapper.findAll(".physical-shelf-row").map((row) => row.findAll(".book-spine").map((book) => book.text()));
}

function rowObjects(wrapper: VueWrapper): string[][] {
  return wrapper.findAll(".physical-shelf-row").map((row) => row.findAll(".shelf-object").map((object) => object.attributes("data-id")!));
}

async function render(layout: Layout = saved, realDraggable = false) {
  myShelf.mockResolvedValue({ items: structuredClone(books), favorites: [], timezone: "Europe/Berlin" });
  physicalShelfLayout.mockResolvedValue(structuredClone(layout));
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(PhysicalShelfPage, { global: { plugins: [pinia], stubs: { VueDraggable: realDraggable ? false : draggable } } });
  await flushPromises();
  return wrapper;
}

describe("physical bookshelf", () => {
  beforeEach(() => {
    myShelf.mockReset();
    physicalShelfLayout.mockReset();
    savePhysicalShelfLayout.mockReset().mockImplementation(async (layout: Layout) => structuredClone(layout));
    patchShelf.mockReset();
  });

  it("restores the saved rows and book order without writing during page load", async () => {
    const wrapper = await render();
    const rows = rowBooks(wrapper);
    expect(rows[0][0]).toContain("Finished");
    expect(rows[0][1]).toContain("First");
    expect(rows[1][0]).toContain("Reading");
    expect(rows[2]).toEqual([]);
    expect(wrapper.get('.material-choice[data-material="walnut"]').attributes("aria-pressed")).toBe("true");
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("prevents edits if loading the saved layout failed, so a default cannot overwrite it", async () => {
    myShelf.mockResolvedValue({ items: books, favorites: [], timezone: "Europe/Berlin" });
    physicalShelfLayout.mockRejectedValue(new Error("offline"));
    const pinia = createPinia();
    setActivePinia(pinia);
    const wrapper = mount(PhysicalShelfPage, { global: { plugins: [pinia], stubs: { VueDraggable: draggable } } });
    await flushPromises();
    const add = wrapper.find(".add-shelf-row");
    expect(!add.exists() || add.attributes("disabled") !== undefined).toBe(true);
    expect(wrapper.findAll(".book-spine")).toHaveLength(0);
    expect(wrapper.text()).toMatch(/load|laden|offline/i);
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("moves books with the keyboard controls across rows and leaves reading status untouched", async () => {
    const wrapper = await render();
    await wrapper.findAll(".book-spine")[0].trigger("click");
    await wrapper.get(".edit-spine").trigger("click");
    await wrapper.get(".move-book-right").trigger("click");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[0][0].rows).toEqual([[1, 2], [3], []]);
    await wrapper.get(".move-book-row").setValue("2");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[1][0].rows).toEqual([[1], [3], [2]]);
    expect(patchShelf).not.toHaveBeenCalled();
    expect(wrapper.get(".spine-sheet-book .finer").text()).toBe(statusLabel("finished"));
    expect(rowBooks(wrapper)[2][0]).toContain("Finished");
    wrapper.unmount();
  });

  it("rolls back a failed save and blocks overlapping layout changes", async () => {
    const wrapper = await render();
    let reject!: (error: Error) => void;
    savePhysicalShelfLayout.mockReturnValueOnce(new Promise((_resolve, fail) => { reject = fail; }));
    await wrapper.get('.material-choice[data-material="white"]').trigger("click");
    expect(wrapper.get('.material-choice[data-material="white"]').attributes("aria-pressed")).toBe("true");
    expect(wrapper.get(".add-shelf-row").attributes("disabled")).toBeDefined();
    await wrapper.get(".add-shelf-row").trigger("click");
    expect(savePhysicalShelfLayout).toHaveBeenCalledTimes(1);
    reject(new Error("offline"));
    await flushPromises();
    expect(wrapper.get('.material-choice[data-material="walnut"]').attributes("aria-pressed")).toBe("true");
    expect(wrapper.get(".add-shelf-row").attributes("disabled")).toBeUndefined();
    expect(rowBooks(wrapper)).toHaveLength(3);
    wrapper.unmount();
  });

  it("keeps spine edits local until they are applied, then persists colour and width together", async () => {
    const wrapper = await render();
    await wrapper.findAll(".book-spine")[0].trigger("click");
    await wrapper.get(".edit-spine").trigger("click");
    await wrapper.get(".spine-color").setValue("#aabbcc");
    await wrapper.get(".spine-width").setValue("52");
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
    await wrapper.get(".spine-editor").trigger("submit");
    await flushPromises();
    expect(savePhysicalShelfLayout).toHaveBeenCalledTimes(1);
    expect(savePhysicalShelfLayout.mock.calls[0][0].spines["2"]).toMatchObject({ color: "#aabbcc", width: 52 });
    expect(savePhysicalShelfLayout.mock.calls[0][0].rows).toEqual(saved.rows);
    wrapper.unmount();
  });

  it("persists a dragged book into the destination row and restores both rows if the save fails", async () => {
    const wrapper = await render();
    const rows = wrapper.findAllComponents(draggable);
    rows[0].vm.$emit("start");
    rows[0].vm.$emit("update:modelValue", [1]);
    rows[2].vm.$emit("update:modelValue", [2]);
    savePhysicalShelfLayout.mockRejectedValueOnce(new Error("offline"));
    rows[0].vm.$emit("end");
    await flushPromises();
    expect(savePhysicalShelfLayout).toHaveBeenCalledTimes(1);
    expect(savePhysicalShelfLayout.mock.calls[0][0].rows).toEqual([[1], [3], [2]]);
    expect(rowBooks(wrapper)[0][0]).toContain("Finished");
    expect(rowBooks(wrapper)[0][1]).toContain("First");
    expect(rowBooks(wrapper)[2]).toEqual([]);
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("removes occupied boards without losing objects, and keeps the saved single board on reload", async () => {
    const wrapper = await render(decorated);
    await wrapper.findAll(".remove-shelf-row")[1].trigger("click");
    await flushPromises();
    const merged: Layout = savePhysicalShelfLayout.mock.calls[0][0];
    expect(merged.rows).toEqual([[2, "decor:plant-1", 1, 3, "decor:frame-1"], []]);
    expect(merged.decorations).toEqual(decorated.decorations);
    expect(merged.spines).toEqual(saved.spines);
    expect(rowObjects(wrapper)).toEqual([["2", "decor:plant-1", "1", "3", "decor:frame-1"], []]);
    await wrapper.findAll(".remove-shelf-row")[1].trigger("click");
    await flushPromises();
    const reduced: Layout = savePhysicalShelfLayout.mock.calls[1][0];
    expect(reduced.rows).toHaveLength(1);
    wrapper.unmount();

    const restored = await render(reduced);
    expect(rowObjects(restored)).toEqual([["2", "decor:plant-1", "1", "3", "decor:frame-1"]]);
    expect(restored.get(".remove-shelf-row").attributes("disabled")).toBeDefined();
    await restored.get(".remove-shelf-row").trigger("click");
    expect(savePhysicalShelfLayout).toHaveBeenCalledTimes(2);
    restored.unmount();
  });

  it("restores an occupied board and its decoration after a failed removal", async () => {
    const wrapper = await render(decorated);
    let reject!: (error: Error) => void;
    savePhysicalShelfLayout.mockReturnValueOnce(new Promise((_resolve, fail) => { reject = fail; }));
    await wrapper.findAll(".remove-shelf-row")[0].trigger("click");
    expect(rowObjects(wrapper)).toHaveLength(2);
    expect(wrapper.get(".add-decoration").attributes("disabled")).toBeDefined();
    expect(wrapper.findAll(".remove-shelf-row").every((button) => button.attributes("disabled") !== undefined)).toBe(true);
    reject(new Error("offline"));
    await flushPromises();
    expect(rowObjects(wrapper)).toEqual(decorated.rows.map((row) => row.map(String)));
    expect(wrapper.get(".add-decoration").attributes("disabled")).toBeUndefined();
    expect(savePhysicalShelfLayout.mock.calls[0][0].decorations).toEqual(decorated.decorations);
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows the book first and only opens the styling controls on request", async () => {
    const wrapper = await render();
    await wrapper.findAll(".book-spine")[0].trigger("click");
    expect(wrapper.get(".spine-sheet-book").text()).toContain("Finished");
    expect(wrapper.get(".open-book").attributes("href")).toBe("/book/OL2W");
    expect(wrapper.find(".spine-editor").exists()).toBe(false);
    await wrapper.get(".edit-spine").trigger("click");
    expect(wrapper.find(".spine-editor").exists()).toBe(true);
    await wrapper.get(".back-to-book").trigger("click");
    expect(wrapper.find(".spine-editor").exists()).toBe(false);
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("stores how a book is placed and lets colour and thickness follow the book again", async () => {
    const wrapper = await render();
    // Book 2 has a hand-picked colour and width; book 1 has none.
    await wrapper.get('.book-spine[data-id="2"]').trigger("click");
    await wrapper.get(".edit-spine").trigger("click");
    await wrapper.get('.spine-display [data-display="flat"]').trigger("click");
    await wrapper.get(".reset-spine-color").trigger("click");
    await wrapper.get(".reset-spine-width").trigger("click");
    await wrapper.get(".spine-editor").trigger("submit");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[0][0].spines).toEqual({ "2": { display: "flat" } });
    expect(wrapper.get('.shelf-object[data-id="2"]').attributes("data-display")).toBe("flat");

    await wrapper.get('.book-spine[data-id="1"]').trigger("click");
    await wrapper.get(".edit-spine").trigger("click");
    await wrapper.get('.spine-display [data-display="cover"]').trigger("click");
    await wrapper.get(".spine-editor").trigger("submit");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[1][0].spines).toEqual({ "1": { display: "cover" }, "2": { display: "flat" } });
    expect(wrapper.get('.shelf-object[data-id="1"]').classes()).toContain("book-face");

    // Back to a plain upright spine: nothing is left to store for the book.
    await wrapper.get('.shelf-object[data-id="1"]').trigger("click");
    await wrapper.get(".edit-spine").trigger("click");
    await wrapper.get('.spine-display [data-display="spine"]').trigger("click");
    await wrapper.get(".spine-editor").trigger("submit");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[2][0].spines).toEqual({ "2": { display: "flat" } });
    wrapper.unmount();
  });

  it("switches the string lights and the black finish without touching the rows", async () => {
    const wrapper = await render();
    expect(wrapper.find(".shelf-lights").exists()).toBe(false);
    await wrapper.get(".toggle-lights").trigger("click");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[0][0]).toMatchObject({ lights: true, rows: saved.rows });
    expect(wrapper.find(".shelf-lights").exists()).toBe(true);
    expect(wrapper.get(".toggle-lights").attributes("aria-pressed")).toBe("true");
    await wrapper.get('.material-choice[data-material="black"]').trigger("click");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[1][0]).toMatchObject({ lights: true, material: "black" });
    wrapper.unmount();
  });

  it.each<DecorationKind>(["plant", "vase", "candle", "frame", "bookend", "globe"])("adds a %s decoration to the chosen board and persists its identity", async (kind) => {
    const wrapper = await render();
    await wrapper.get(".add-decoration").trigger("click");
    expect(wrapper.findAll(".decoration-choice")).toHaveLength(6);
    await wrapper.get(".decoration-row").setValue("1");
    await wrapper.get(`.decoration-choice[data-kind="${kind}"]`).trigger("click");
    await flushPromises();
    const submitted: Layout = savePhysicalShelfLayout.mock.calls[0][0];
    const [id] = Object.keys(submitted.decorations!);
    expect(id).toMatch(/^decor:[\da-f-]+$/i);
    expect(submitted.decorations![id]).toMatchObject({ kind, color: expect.stringMatching(/^#[\da-f]{6}$/i) });
    expect(submitted.rows).toEqual([[2, 1], [3, id], []]);
    expect(submitted.spines).toEqual(saved.spines);
    expect(wrapper.find(".decoration-row").exists()).toBe(false);
    expect(wrapper.get(`.shelf-decoration[data-kind="${kind}"]`).attributes("data-id")).toBe(id);
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("restores the existing objects and decoration records when adding decoration fails", async () => {
    const wrapper = await render(decorated);
    await wrapper.get(".add-decoration").trigger("click");
    await wrapper.get(".decoration-row").setValue("2");
    savePhysicalShelfLayout.mockRejectedValueOnce(new Error("offline"));
    await wrapper.get('.decoration-choice[data-kind="vase"]').trigger("click");
    await flushPromises();
    expect(Object.keys(savePhysicalShelfLayout.mock.calls[0][0].decorations)).toHaveLength(3);
    expect(rowObjects(wrapper)).toEqual(decorated.rows.map((row) => row.map(String)));
    expect(wrapper.findAll(".shelf-decoration")).toHaveLength(2);
    expect(wrapper.find('.shelf-decoration[data-kind="vase"]').exists()).toBe(false);
    expect(wrapper.get(".add-decoration").attributes("disabled")).toBeUndefined();
    wrapper.unmount();
  });

  it("persists a decoration dragged between mixed rows together with its appearance", async () => {
    const wrapper = await render(decorated);
    const rows = wrapper.findAllComponents(draggable);
    rows[0].vm.$emit("start");
    rows[0].vm.$emit("update:modelValue", [2, 1]);
    rows[1].vm.$emit("update:modelValue", [3, "decor:plant-1", "decor:frame-1"]);
    rows[0].vm.$emit("end");
    await flushPromises();
    const submitted: Layout = savePhysicalShelfLayout.mock.calls[0][0];
    expect(submitted.rows).toEqual([[2, 1], [3, "decor:plant-1", "decor:frame-1"], []]);
    expect(submitted.decorations).toEqual(decorated.decorations);
    expect(submitted.spines).toEqual(saved.spines);
    expect(rowObjects(wrapper)).toEqual(submitted.rows.map((row) => row.map(String)));
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps exactly one decoration node after the real drag library restores and updates mixed rows", async () => {
    const wrapper = await render(decorated, true);
    const rows = wrapper.findAllComponents(VueDraggable);
    const source = rows[0].element as HTMLElement;
    const destination = rows[1].element as HTMLElement;
    const moving = wrapper.get('.shelf-decoration[data-id="decor:plant-1"]').element as HTMLElement;
    const sourceDrag = rows[0].vm as unknown as UseDraggableReturn;
    const destinationDrag = rows[1].vm as unknown as UseDraggableReturn;
    const event = {
      item: moving, from: source, to: destination,
      oldIndex: 1, oldDraggableIndex: 1, newIndex: 1, newDraggableIndex: 1,
      clone: moving.cloneNode(true), pullMode: true,
    } as unknown as DraggableEvent<ShelfObjectId>;

    // Sortable moves the DOM node, then the library restores it for Vue's keyed update.
    sourceDrag.option("onStart")!(event);
    destination.insertBefore(moving, destination.children[1]);
    destinationDrag.option("onAdd")!(event);
    sourceDrag.option("onRemove")!(event);
    sourceDrag.option("onEnd")!(event);
    await flushPromises();

    expect(savePhysicalShelfLayout).toHaveBeenCalledTimes(1);
    expect(savePhysicalShelfLayout.mock.calls[0][0].rows).toEqual([[2, 1], [3, "decor:plant-1", "decor:frame-1"], []]);
    expect(rowObjects(wrapper)).toEqual([["2", "1"], ["3", "decor:plant-1", "decor:frame-1"], []]);
    expect(wrapper.findAll('.shelf-decoration[data-id="decor:plant-1"]')).toHaveLength(1);
    expect(wrapper.findAll(".book-spine")).toHaveLength(3);
    wrapper.unmount();
  });

  it("edits decoration color locally before saving it without changing books or other decorations", async () => {
    const wrapper = await render(decorated);
    await wrapper.get('.shelf-decoration[data-id="decor:plant-1"]').trigger("click");
    expect(wrapper.find(".decoration-editor").exists()).toBe(true);
    await wrapper.get(".decoration-color").setValue("#aabbcc");
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
    await wrapper.get(".decoration-editor").trigger("submit");
    await flushPromises();
    const submitted: Layout = savePhysicalShelfLayout.mock.calls[0][0];
    expect(submitted.decorations).toEqual({ ...decorated.decorations, "decor:plant-1": { kind: "plant", color: "#aabbcc" } });
    expect(submitted.rows).toEqual(decorated.rows);
    expect(submitted.spines).toEqual(saved.spines);
    expect(wrapper.find(".decoration-editor").exists()).toBe(false);
    wrapper.unmount();
  });

  it("moves decoration through the accessible position controls", async () => {
    const wrapper = await render(decorated);
    await wrapper.get('.shelf-decoration[data-id="decor:plant-1"]').trigger("click");
    expect(wrapper.get(".move-book-row").attributes("aria-label")).toBeTruthy();
    await wrapper.get(".move-book-right").trigger("click");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[0][0].rows).toEqual([[2, 1, "decor:plant-1"], [3, "decor:frame-1"], []]);
    await wrapper.get(".move-book-row").setValue("2");
    await flushPromises();
    expect(savePhysicalShelfLayout.mock.calls[1][0].rows).toEqual([[2, 1], [3, "decor:frame-1"], ["decor:plant-1"]]);
    expect(savePhysicalShelfLayout.mock.calls[1][0].decorations).toEqual(decorated.decorations);
    expect(rowObjects(wrapper)[2]).toEqual(["decor:plant-1"]);
    wrapper.unmount();
  });

  it("removes decoration while preserving every book, book style and other decoration", async () => {
    const wrapper = await render(decorated);
    await wrapper.get('.shelf-decoration[data-id="decor:plant-1"]').trigger("click");
    await wrapper.get(".remove-decoration").trigger("click");
    await flushPromises();
    const submitted: Layout = savePhysicalShelfLayout.mock.calls[0][0];
    expect(submitted.rows).toEqual([[2, 1], [3, "decor:frame-1"], []]);
    expect(submitted.decorations).toEqual({ "decor:frame-1": decorated.decorations!["decor:frame-1"] });
    expect(submitted.spines).toEqual(saved.spines);
    expect(wrapper.find(".decoration-editor").exists()).toBe(false);
    expect(wrapper.findAll(".book-spine")).toHaveLength(3);
    expect(wrapper.findAll(".shelf-decoration")).toHaveLength(1);
    expect(patchShelf).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
