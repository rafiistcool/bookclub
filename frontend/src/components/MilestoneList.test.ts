import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MilestoneOut } from "../types";

const milestones = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { milestones: (...args: unknown[]) => milestones(...args) },
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

async function mountList(canManage: boolean) {
  setActivePinia(createPinia());
  const wrapper = mount(MilestoneList, {
    props: { slug: "readers-ab12", pickId: 5, canManage, timezone: "Europe/Berlin" },
    global: { plugins: [i18n] },
  });
  await flushPromises();
  return wrapper;
}

describe("MilestoneList", () => {
  beforeEach(() => milestones.mockReset());

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

  it("keeps remove quiet until it asks to be confirmed", async () => {
    milestones.mockResolvedValue({ items: [milestone()] });
    const wrapper = await mountList(true);
    const remove = wrapper.get(".milestone .text-btn");
    expect(remove.classes()).toContain("quiet");
    await remove.trigger("click");
    expect(remove.classes()).toEqual(expect.arrayContaining(["danger"]));
    expect(remove.classes()).not.toContain("quiet");
    expect(remove.text()).toBe(i18n.global.t("common.deleteConfirm"));
  });
});
