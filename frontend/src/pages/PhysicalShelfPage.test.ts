import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { statusLabel } from "../constants";
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
  rows: number[][];
  material: "oak" | "walnut" | "white";
  spines: Record<string, { color?: string; width?: number }>;
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

function rowBooks(wrapper: VueWrapper): string[][] {
  return wrapper.findAll(".physical-shelf-row").map((row) => row.findAll(".book-spine").map((book) => book.text()));
}

async function render(layout: Layout = saved) {
  myShelf.mockResolvedValue({ items: structuredClone(books), favorites: [], timezone: "Europe/Berlin" });
  physicalShelfLayout.mockResolvedValue(structuredClone(layout));
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(PhysicalShelfPage, { global: { plugins: [pinia], stubs: { VueDraggable: draggable } } });
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
});
