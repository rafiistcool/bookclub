import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MilestoneOut } from "../types";

const milestones = vi.fn();
const deleteMilestone = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    milestones: (...args: unknown[]) => milestones(...args),
    deleteMilestone: (...args: unknown[]) => deleteMilestone(...args),
  },
}));

import { i18n } from "../i18n";
import MilestoneList from "./MilestoneList.vue";

function milestone(overrides: Partial<MilestoneOut> = {}): MilestoneOut {
  return {
    id: 1,
    pick_id: 5,
    title: "Part one",
    unit: null,
    range_from: null,
    range_to: null,
    due_at: null,
    due_local: null,
    due_label: null,
    position: 0,
    note_count: 0,
    passed: false,
    ...overrides,
  };
}

async function mountList(canManage: boolean, attach = false) {
  setActivePinia(createPinia());
  const wrapper = mount(MilestoneList, {
    props: { slug: "readers-ab12", pickId: 5, canManage, timezone: "Europe/Berlin" },
    global: { plugins: [i18n] },
    ...(attach ? { attachTo: document.body } : {}),
  });
  await flushPromises();
  return wrapper;
}

describe("MilestoneList", () => {
  beforeEach(() => {
    milestones.mockReset();
    deleteMilestone.mockReset();
  });

  it("renders nothing for a member while the pick has no milestones", async () => {
    milestones.mockResolvedValue({ items: [] });
    const wrapper = await mountList(false);
    expect(milestones).toHaveBeenCalledWith("readers-ab12", 5);
    expect(wrapper.find(".milestones").exists()).toBe(false);
    expect(wrapper.text()).toBe("");
  });

  it("shows a member the milestones there are, without the admin buttons", async () => {
    milestones.mockResolvedValue({ items: [milestone()] });
    const wrapper = await mountList(false);
    expect(wrapper.get("#milestones-heading").text()).toBe("Milestones");
    expect(wrapper.findAll(".milestone").map((row) => row.get("strong").text())).toEqual(["Part one"]);
    expect(wrapper.find(".text-btn").exists()).toBe(false);
  });

  it("gives an admin the heading, a short example and the add button when there are none", async () => {
    milestones.mockResolvedValue({ items: [] });
    const wrapper = await mountList(true);
    expect(wrapper.find(".milestones").exists()).toBe(true);
    expect(wrapper.get(".milestones > .fine").text()).toBe(i18n.global.t("milestones.emptyAdmin"));
    expect(wrapper.get(".section-head .text-btn").text()).toBe(i18n.global.t("milestones.add"));
  });

  it("splits the meta line into facts that each wrap as a unit", async () => {
    milestones.mockResolvedValue({
      items: [milestone({ unit: "chapter", range_from: 1, range_to: 5, note_count: 2 })],
    });
    const wrapper = await mountList(false);
    const facts = wrapper.findAll(".milestone-facts > span").map((part) => part.text());
    expect(facts).toEqual([
      i18n.global.t("milestones.chapterRange", { from: 1, to: 5 }),
      i18n.global.t("milestones.notes", 2),
    ]);
    expect(facts.join("")).not.toContain("·");
  });

  it("shows a quiet remove only in edit mode, and asks before it removes", async () => {
    milestones.mockResolvedValue({ items: [milestone(), milestone({ id: 2, title: "Part two" })] });
    const wrapper = await mountList(true);
    expect(wrapper.find(".milestone-remove").exists()).toBe(false);
    const edit = wrapper.findAll(".section-head .text-btn").find((button) => button.text() === i18n.global.t("common.edit"))!;
    await edit.trigger("click");
    // The label says the state ("Done"); a pressed state on top would read "Done, pressed".
    expect(edit.attributes("aria-pressed")).toBeUndefined();
    expect(edit.text()).toBe(i18n.global.t("milestones.doneEditing"));
    const removes = wrapper.findAll(".milestone-remove");
    expect(removes).toHaveLength(2);
    expect(removes[0].text()).toBe(i18n.global.t("common.remove"));
    expect(removes[0].classes()).not.toContain("danger");
    // The button names the stop it removes for a screen reader.
    expect(removes[0].attributes("aria-describedby")).toBe(wrapper.findAll(".milestone strong")[0].attributes("id"));
    await removes[0].trigger("click");
    expect(removes[0].classes()).toContain("danger");
    expect(removes[0].text()).toBe(i18n.global.t("milestones.removeConfirm"));
  });

  it("moves focus on to the next stop's remove, and to Add once the last one is gone", async () => {
    const first = milestone();
    const second = milestone({ id: 2, title: "Part two" });
    milestones.mockResolvedValue({ items: [first, second] });
    const wrapper = await mountList(true, true);
    await wrapper.findAll(".section-head .text-btn").find((button) => button.text() === i18n.global.t("common.edit"))!.trigger("click");
    const confirmRemove = async (index: number) => {
      const button = wrapper.findAll(".milestone-remove")[index];
      (button.element as HTMLElement).focus();
      await button.trigger("click");
      await button.trigger("click");
      await flushPromises();
    };

    deleteMilestone.mockResolvedValue(undefined);
    milestones.mockResolvedValue({ items: [second] });
    await confirmRemove(0);
    expect(deleteMilestone).toHaveBeenCalledWith("readers-ab12", 5, 1);
    expect(document.activeElement).toBe(wrapper.get(".milestone-remove").element);
    expect(document.activeElement?.getAttribute("aria-describedby")).toBe("milestone-2");

    milestones.mockResolvedValue({ items: [] });
    await confirmRemove(0);
    // Edit mode ends with the list; focus lands on "Add", not on the body.
    expect(document.activeElement).toBe(wrapper.get(".milestone-add").element);
    expect(document.activeElement?.textContent?.trim()).toBe(i18n.global.t("milestones.add"));
    wrapper.unmount();
  });

  it("keeps a member's list free of edit tools", async () => {
    milestones.mockResolvedValue({ items: [milestone()] });
    const wrapper = await mountList(false);
    expect(wrapper.find(".milestone-tools").exists()).toBe(false);
    expect(wrapper.find(".milestone-remove").exists()).toBe(false);
  });
});
