import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ShelfGroup from "./ShelfGroup.vue";
import { shelfGroups } from "../shelf";
import type { ShelfItem } from "../types";

const item = (id: number, position: number, finished_at: string): ShelfItem => ({
  id, position, status: "finished", finished_at, updated_at: "", rating: null,
  take: "", dnf_reason: "", progress: null,
  book: { id, title: `Book ${id}`, authors: "", cover_id: null, cover_url: null, year: null, ol_work_key: `/works/OL${id}W` },
});
const draggable = {
  name: "VueDraggable", props: ["modelValue", "group", "disabled", "handle"],
  emits: ["update:modelValue", "update", "add"], template: '<div><slot /></div>',
};
const items = [item(1, 0, "2026-07-02T12:00:00Z"), item(2, 1, "2026-08-02T12:00:00Z"), item(3, 2, "2026-07-04T12:00:00Z")];

function render(extra = {}) {
  const group = shelfGroups(items, "UTC").find((group) => group.month === "2026-07")!;
  return mount(ShelfGroup, { props: { group, items, timezone: "UTC", ...extra }, global: { stubs: { VueDraggable: draggable } } });
}

describe("shelf drag and read-only controls", () => {
  it("translates a month-local drag to a persisted position", async () => {
    const wrapper = render();
    const drag = wrapper.getComponent(draggable);
    drag.vm.$emit("update:modelValue", [items[2], items[0]]);
    drag.vm.$emit("update", { newIndex: 0 });
    expect(wrapper.emitted("dropped")?.[0]).toEqual([items[2], "finished", 0]);
    wrapper.unmount();
  });

  it("rejects dragging finished books into a different month", () => {
    const wrapper = render();
    const group = wrapper.getComponent(draggable).props("group");
    const element = document.createElement("article");
    element.dataset.id = "2";
    expect(group.put(null, null, element)).toBe(false);
    element.dataset.id = "1";
    expect(group.put(null, null, element)).toBe(true);
    wrapper.unmount();
  });

  it("requires mobile organize mode and disables all editing for member shelves", async () => {
    const wrapper = render({ grid: true });
    expect(wrapper.getComponent(draggable).props("disabled")).toBe(true);
    await wrapper.setProps({ organizing: true });
    expect(wrapper.getComponent(draggable).props("disabled")).toBe(false);
    expect(wrapper.getComponent(draggable).props("handle")).toBe(".drag-handle");
    await wrapper.setProps({ readonly: true });
    expect(wrapper.getComponent(draggable).props("disabled")).toBe(true);
    expect(wrapper.find("button").exists()).toBe(false);
    wrapper.unmount();
  });
});
