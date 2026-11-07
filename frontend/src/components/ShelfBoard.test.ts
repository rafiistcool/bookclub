import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ShelfBoard from "./ShelfBoard.vue";
import ShelfGroup from "./ShelfGroup.vue";
import { shelfItem } from "../test/fixtures";
import type { ShelfItem } from "../types";

const item = (id: number, status: ShelfItem["status"], finished_at?: string): ShelfItem =>
  shelfItem({ id, position: id, status, finished_at: finished_at ?? null });
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
    const wrapper = render({ clubPickIds: [2] });
    expect(wrapper.findAll(".month-heading").map(node => node.text())).toEqual(["September 2026 1", "August 2026 1"]);
    expect(wrapper.findAll(".shelf-status").map(node => node.text())).toEqual(["To read", "Reading", "Read", "Read"]);
    expect(wrapper.find(".club-pick").exists()).toBe(true);
    wrapper.unmount();
  });
  it("heads the unfinished books apart from the status words, and reads a tile title, author, stars", () => {
    const reading = { ...item(2, "currently_reading"), progress: 35 };
    const rated = shelfItem({ id: 5, position: 5, status: "finished", rating: 4, finished_at: "2026-09-12T12:00:00Z",
      book: { id: 5, title: "Middlemarch", authors: "George Eliot" } });
    const wrapper = mount(ShelfBoard, { props: { items: [reading, rated] }, global: { stubs: { VueDraggable: draggable } } });
    expect(wrapper.get(".shelf-section-heading").text()).toBe("Not finished yet");
    // The progress uses the locale's percent, like the progress sheet ("35%" in English, "35 %" in German).
    expect(wrapper.findAll(".shelf-status")[0]!.get(".nums").text()).toBe("35%");
    const tile = wrapper.findAll(".book-tile")[1]!;
    const lines = tile.findAll(".book-tile-title, .book-tile-sub").map(node => node.classes().includes("stars") ? "stars" : node.text());
    expect(lines).toEqual(["Middlemarch", "George Eliot", "stars"]);
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
