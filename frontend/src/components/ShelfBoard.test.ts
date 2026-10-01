import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ShelfBoard from "./ShelfBoard.vue";
import ShelfGroup from "./ShelfGroup.vue";
import type { ShelfItem } from "../types";

const item = (id: number, status: ShelfItem["status"], finished_at?: string): ShelfItem => ({
  id, position: id, status, finished_at, updated_at: "", rating: null, take: "", dnf_reason: "", progress: null,
  book: { id, title: `Book ${id}`, authors: "", cover_id: null, cover_url: null, year: null, ol_work_key: `/works/OL${id}W` },
});
const draggable = {
  name: "VueDraggable", props: ["modelValue", "disabled", "handle", "delay", "delayOnTouchOnly", "group"],
  emits: ["update:modelValue", "update", "add"], template: '<div><slot /></div>',
};
const september = item(3, "finished", "2026-09-12T12:00:00Z");
const august = item(4, "finished", "2026-08-21T12:00:00Z");
const items = [item(1, "want_to_read"), item(2, "currently_reading"), september, august];
function render(extra = {}) {
  return mount(ShelfBoard, { props: { items, ...extra }, global: { stubs: { VueDraggable: draggable } } });
}

describe("monthly shelf", () => {
  it("shows active books together and completed months newest first in the all view", () => {
    const wrapper = render({ clubPickKey: "/works/OL2W" });
    expect(wrapper.findAll(".month-heading").map(node => node.text())).toEqual(["September 2026 1", "August 2026 1"]);
    expect(wrapper.findAll(".shelf-status").map(node => node.text())).toEqual(["Want", "Reading", "Done", "Done"]);
    expect(wrapper.find(".club-pick").exists()).toBe(true);
    wrapper.unmount();
  });
  it("keeps direct touch-delayed dragging and separates unfinished books from month targets", () => {
    const wrapper = render();
    const groups = wrapper.findAllComponents(draggable);
    expect(groups[0].props("handle")).toBeUndefined();
    expect(groups[0].props("delay")).toBe(250);
    expect(groups[0].props("delayOnTouchOnly")).toBe(true);
    expect(groups[0].props("group")).not.toBe(groups[1].props("group"));
    expect(groups[1].props("group")).toBe(groups[2].props("group"));
    wrapper.unmount();
  });
  it("emits the target month even when a cross-month drop has identical old/new indices", () => {
    const wrapper = render({ filter: "finished" });
    const destination = wrapper.findAllComponents(ShelfGroup).find(group => group.props("month") === "2026-08")!;
    const drag = destination.getComponent(draggable);
    drag.vm.$emit("update:modelValue", [september, august]);
    drag.vm.$emit("add", { oldIndex: 0, newIndex: 0 });
    expect(wrapper.emitted("reordered")?.[0]).toEqual([september, 2, "2026-08"]);
    wrapper.unmount();
  });
  it("provides an empty adjacent month, and allows adding an older month across years", async () => {
    const wrapper = render({ items: [september], filter: "finished" });
    expect(wrapper.findAll(".month-heading").map(node => node.text())).toEqual(["September 2026 1", "August 2026 0"]);
    const empty = wrapper.findAllComponents(ShelfGroup)[1].getComponent(draggable);
    empty.vm.$emit("update:modelValue", [september]);
    empty.vm.$emit("add", { oldIndex: 0, newIndex: 0 });
    expect(wrapper.emitted("reordered")?.[0]).toEqual([september, 0, "2026-08"]);
    await wrapper.get(".month-picker button").trigger("click");
    await wrapper.get('input[type="month"]').setValue("2025-12");
    await wrapper.get("form").trigger("submit");
    expect(wrapper.findAll(".month-heading").map(node => node.text())).toContain("December 2025 0");
    wrapper.unmount();
  });
  it("uses the club timezone and keeps member shelves read-only without empty drop targets", () => {
    const wrapper = render({ items: [item(1, "finished", "2025-12-31T23:30:00")], timezone: "Europe/Berlin", readonly: true });
    expect(wrapper.get(".month-heading").text()).toBe("January 2026 1");
    expect(wrapper.getComponent(draggable).props("disabled")).toBe(true);
    expect(wrapper.find("button").exists()).toBe(false);
    expect(wrapper.find(".empty-month").exists()).toBe(false);
    wrapper.unmount();
  });
});
