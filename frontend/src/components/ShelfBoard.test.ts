import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ShelfBoard from "./ShelfBoard.vue";
import ShelfGroup from "./ShelfGroup.vue";
import { i18n } from "../i18n";
import { shelfItem } from "../test/fixtures";
import type { ShelfItem } from "../types";

const item = (id: number, status: ShelfItem["status"], finished_at?: string): ShelfItem =>
  shelfItem({ id, position: id, status, finished_at: finished_at ?? null });
const draggable = {
  name: "VueDraggable", props: ["modelValue", "disabled", "handle", "delay", "delayOnTouchOnly", "group", "fallbackClass"],
  emits: ["update:modelValue", "update", "add", "start", "change", "end"], template: '<div><slot /></div>',
};
const september = item(3, "finished", "2026-09-12T12:00:00Z");
const august = item(4, "finished", "2026-08-21T12:00:00Z");
const items = [item(1, "want_to_read"), item(2, "currently_reading"), september, august];
function render(extra = {}) {
  return mount(ShelfBoard, { props: { items, ...extra }, global: { stubs: { VueDraggable: draggable } } });
}
const markers = (wrapper: ReturnType<typeof render>) =>
  wrapper.findAll(".month-marker").map(node => node.text()).filter(Boolean);

describe("the shelf's grids", () => {
  it("shows the unfinished books together, then every finished book in one grid with month markers", () => {
    const wrapper = render({ clubPickIds: [2] });
    expect(wrapper.findAll(".shelf-section-heading").map(node => node.text())).toEqual(["Not finished yet", "Finished 2"]);
    expect(wrapper.findAll(".shelf-status").map(node => node.text())).toEqual(["Planned", "Reading", "Finished", "Finished"]);
    // One grid for the finished books, not one section per month, and no "add a month".
    expect(wrapper.findAllComponents(ShelfGroup)).toHaveLength(2);
    expect(wrapper.find(".shelf-month").exists()).toBe(false);
    expect(wrapper.find(".month-picker").exists()).toBe(false);
    expect(markers(wrapper)).toEqual(["Sep 2026", "Aug 2026"]);
    expect(wrapper.find(".club-pick").exists()).toBe(true);
    wrapper.unmount();
  });

  it("names each status chip by its visible words first (WCAG 2.5.3)", () => {
    const reading = { ...item(2, "currently_reading"), progress: 35, book: { ...item(2, "currently_reading").book, title: "Circe" } };
    const wrapper = render({ items: [reading, september] });
    for (const chip of wrapper.findAll("button.shelf-status")) {
      // The percent sits a margin apart on screen, so spaces do not count.
      const shown = chip.get(".status-label").text().replace(/\s/g, "");
      const label = chip.attributes("aria-label")!;
      expect(label.split(" – ")[0]!.replace(/\s/g, ""), label).toBe(shown);
    }
    expect(wrapper.get("button.shelf-status").attributes("aria-label")).toBe("Reading 35% – change the status of “Circe”");
    wrapper.unmount();
  });

  it("marks only the first book of a month, keeps the line on the others, and puts undated books last", () => {
    const early = item(5, "finished", "2026-09-02T12:00:00Z");
    const undated = item(6, "finished");
    const wrapper = render({ items: [undated, september, early, august], filter: "finished" });
    const cards = wrapper.findAll(".shelf-card");
    expect(cards.map(card => card.get(".month-marker").text())).toEqual(["Sep 2026", "", "Aug 2026", "No date"]);
    expect(cards.map(card => card.get(".month-marker").classes().includes("starts"))).toEqual([true, false, true, true]);
    // Seen, not read out twice: each card says its own month to a screen reader.
    expect(cards[0]!.get(".month-marker").attributes("aria-hidden")).toBe("true");
    expect(cards[1]!.get("time").attributes("aria-label")).toBe("Finished: September 2026");
    // The unfinished cards carry no marker line.
    const all = render();
    expect(all.findAll(".shelf-card").filter(card => card.find(".month-marker").exists())).toHaveLength(2);
    wrapper.unmount();
    all.unmount();
  });

  it("writes the markers short in German", () => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = "de";
    const wrapper = render({ filter: "finished" });
    expect(markers(wrapper)).toEqual(["Sept. 2026", "Aug. 2026"]);
    expect(wrapper.get(".shelf-section-heading").text()).toBe("Gelesen 2");
    i18n.global.locale.value = previous;
    wrapper.unmount();
  });

  it("keeps direct touch-delayed dragging, with unfinished and finished books apart", () => {
    const wrapper = render();
    const groups = wrapper.findAllComponents(draggable);
    expect(groups).toHaveLength(2);
    expect(groups[0]!.props("handle")).toBeUndefined();
    expect(groups[0]!.props("delay")).toBe(250);
    expect(groups[0]!.props("delayOnTouchOnly")).toBe(true);
    expect(groups[0]!.props("group")).not.toBe(groups[1]!.props("group"));
    wrapper.unmount();
  });

  it("moves a book dropped among another month's books into that month", () => {
    const wrapper = render({ filter: "finished" });
    const drag = wrapper.getComponent(draggable);
    // September's book dragged behind August's: it lands in August.
    drag.vm.$emit("update:modelValue", [august, september]);
    drag.vm.$emit("update", { oldIndex: 0, newIndex: 1 });
    expect(wrapper.emitted("reordered")?.[0]).toEqual([september, 3, "2026-08"]);
    wrapper.unmount();
  });

  it("keeps the month of a book that stays beside a book of its own month", () => {
    const early = item(5, "finished", "2026-09-02T12:00:00Z");
    const wrapper = render({ items: [september, early, august], filter: "finished" });
    const drag = wrapper.getComponent(draggable);
    // The order inside September changes; the month does not.
    drag.vm.$emit("update:modelValue", [early, september, august]);
    drag.vm.$emit("update", { oldIndex: 1, newIndex: 0 });
    expect(wrapper.emitted("reordered")?.[0]?.[2]).toBe("2026-09");
    wrapper.unmount();
  });

  it("places a book dropped at the end of its month among that month's books, not before the next month", () => {
    // September's book is first in the manual order (its date was set later);
    // October's books follow it there, but come first in the grid.
    const at = (id: number, iso: string, shelf_position: number) => ({ ...item(id, "finished", iso), shelf_position });
    const sep = at(1, "2026-09-12T12:00:00Z", 0);
    const x = at(2, "2026-10-20T12:00:00Z", 1), a1 = at(3, "2026-10-10T12:00:00Z", 2), a2 = at(4, "2026-10-05T12:00:00Z", 3);
    const wrapper = render({ items: [sep, x, a1, a2], filter: "finished" });
    expect(wrapper.findAll(".shelf-card").map(card => card.attributes("data-id"))).toEqual(["2", "3", "4", "1"]);
    const drag = wrapper.getComponent(draggable);
    drag.vm.$emit("update:modelValue", [a1, a2, x, sep]);
    drag.vm.$emit("update", { oldIndex: 0, newIndex: 2 });
    // After a2 (position 3 without x), not before September's book (0).
    expect(wrapper.emitted("reordered")?.[0]).toEqual([x, 3, "2026-10"]);
    wrapper.unmount();
  });

  it("moves the month markers with the dragged card, so the slot shows the month the drop gives", async () => {
    const at = (id: number, iso: string) => item(id, "finished", iso);
    const octA = at(1, "2026-10-20T12:00:00Z"), octB = at(2, "2026-10-02T12:00:00Z");
    const sepC = at(3, "2026-09-20T12:00:00Z"), sepD = at(4, "2026-09-02T12:00:00Z");
    const wrapper = render({ items: [octA, octB, sepC, sepD], filter: "finished" });
    const marker = (id: number) => wrapper.get(`.shelf-card[data-id="${id}"] .month-marker`).text();
    const drag = wrapper.getComponent(draggable);
    expect([1, 2, 3, 4].map(marker)).toEqual(["Oct 2026", "", "Sep 2026", ""]);
    // October's first book lifted and held between the two September books.
    drag.vm.$emit("start", { oldIndex: 0, oldDraggableIndex: 0 });
    drag.vm.$emit("change", { newIndex: 2, newDraggableIndex: 2 });
    await wrapper.vm.$nextTick();
    expect([1, 2, 3, 4].map(marker)).toEqual(["", "Oct 2026", "Sep 2026", ""]);
    // Let go elsewhere (or nowhere): the markers are the grid's again.
    drag.vm.$emit("end", {});
    await wrapper.vm.$nextTick();
    expect([1, 2, 3, 4].map(marker)).toEqual(["Oct 2026", "", "Sep 2026", ""]);
    // The copy under the finger names no month.
    expect(drag.props("fallbackClass")).toBe("card-lifted");
    wrapper.unmount();
  });

  it("uses the reader's timezone and keeps member shelves read-only", () => {
    const wrapper = render({ items: [item(1, "finished", "2025-12-31T23:30:00")], timezone: "Europe/Berlin", readonly: true });
    expect(markers(wrapper)).toEqual(["Jan 2026"]);
    expect(wrapper.getComponent(draggable).props("disabled")).toBe(true);
    expect(wrapper.find("button").exists()).toBe(false);
    wrapper.unmount();
  });

  it("says nothing is there when a filter leaves no book", () => {
    const wrapper = render({ items: [item(1, "want_to_read")], filter: "finished" });
    expect(wrapper.find(".finished-section").exists()).toBe(false);
    expect(wrapper.text()).toContain("Nothing here.");
    wrapper.unmount();
  });
});
