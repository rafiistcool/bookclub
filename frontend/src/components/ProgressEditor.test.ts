import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { i18n } from "../i18n";
import ProgressEditor from "./ProgressEditor.vue";

function mountEditor(props: { value: number | null; pages?: number | null; busy?: boolean }) {
  return mount(ProgressEditor, {
    props: { label: "Your progress", bookTitle: "Circe", ...props },
    global: { plugins: [i18n] },
  });
}

describe("ProgressEditor", () => {
  it("shows the stored percent as a row and opens the sheet only on a tap", async () => {
    const wrapper = mountEditor({ value: 62 });
    // The stored value as it is, never snapped to a step.
    expect(wrapper.get(".progress-trigger").text()).toContain("62%");
    expect(wrapper.find(".sheet").exists()).toBe(false);
    await wrapper.get(".progress-trigger").trigger("click");
    expect(wrapper.get(".sheet input[type=range]").element).toHaveProperty("value", "62");
  });

  it("steps in percent without a page count and saves only a change", async () => {
    const wrapper = mountEditor({ value: 40 });
    await wrapper.get(".progress-trigger").trigger("click");
    const save = wrapper.get(".sheet .btn-primary");
    expect(save.attributes("disabled")).toBeDefined();
    await wrapper.get('.sheet button[aria-label="Forward 5 percent"]').trigger("click");
    await wrapper.get('.sheet button[aria-label="Forward 1 percent"]').trigger("click");
    await save.trigger("click");
    expect(wrapper.emitted("save")).toEqual([[46]]);
    expect(wrapper.find(".sheet").exists()).toBe(false);
  });

  it("counts in pages when the book has a page count, and saves the percent", async () => {
    const wrapper = mountEditor({ value: 50, pages: 300 });
    expect(wrapper.get(".progress-trigger").text()).toContain("p. 150 of 300");
    await wrapper.get(".progress-trigger").trigger("click");
    const page = wrapper.get(".sheet input[type=number]");
    expect(page.element).toHaveProperty("value", "150");
    await wrapper.get('.sheet button[aria-label="Forward 10 pages"]').trigger("click");
    expect(wrapper.get(".readout-percent").text()).toBe("53%");
    await page.setValue("999");
    // Past the last page is the last page.
    expect(wrapper.get(".readout-percent").text()).toBe("100%");
    await wrapper.get(".sheet .btn-primary").trigger("click");
    expect(wrapper.emitted("save")).toEqual([[100]]);
  });

  it("does not open while a save is under way", async () => {
    const wrapper = mountEditor({ value: 10, busy: true });
    await wrapper.get(".progress-trigger").trigger("click");
    expect(wrapper.find(".sheet").exists()).toBe(false);
  });
});
