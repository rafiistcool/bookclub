import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { shelfItem } from "../test/fixtures";

const patchShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.name = "ApiError";
      this.status = status;
    }
  },
  api: { patchShelf: (...args: unknown[]) => patchShelf(...args) },
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useFinish } from "../stores/finish";
import ProgressEditor from "./ProgressEditor.vue";

function mountEditor(props: {
  value: number | null;
  pages?: number | null;
  busy?: boolean;
  shelfId?: number | null;
  clubSlug?: string | null;
}) {
  return mount(ProgressEditor, {
    props: { label: "Your progress", bookTitle: "Circe", ...props },
    global: { plugins: [i18n] },
  });
}

function buttonText(wrapper: ReturnType<typeof mountEditor>, text: string) {
  return wrapper.findAll(".sheet button").find((node) => node.text() === text);
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

  it("keeps a stored percent that no whole page reaches until the page changes", async () => {
    // 35 % of 50 pages is page 17.5: shown as page 18, still saved as 35 %.
    const wrapper = mountEditor({ value: 35, pages: 50 });
    await wrapper.get(".progress-trigger").trigger("click");
    expect(wrapper.get(".readout-percent").text()).toBe("35%");
    expect(wrapper.get(".sheet .btn-primary").attributes("disabled")).toBeDefined();
    await wrapper.get('.sheet button[aria-label="Forward 1 page"]').trigger("click");
    await wrapper.get('.sheet button[aria-label="Back 1 page"]').trigger("click");
    expect(wrapper.get(".sheet .btn-primary").attributes("disabled")).toBeDefined();
  });

  it("does not open while a save is under way", async () => {
    const wrapper = mountEditor({ value: 10, busy: true });
    await wrapper.get(".progress-trigger").trigger("click");
    expect(wrapper.find(".sheet").exists()).toBe(false);
  });

  describe("at the end of the book", () => {
    beforeEach(() => {
      setActivePinia(createPinia());
      patchShelf.mockReset();
    });

    it("keeps a plain save at the last page without a shelf entry", async () => {
      const wrapper = mountEditor({ value: 95 });
      await wrapper.get(".progress-trigger").trigger("click");
      await wrapper.get('.sheet button[aria-label="Forward 5 percent"]').trigger("click");
      expect(buttonText(wrapper, "Mark as read")).toBeUndefined();
      await wrapper.get(".sheet .btn-primary").trigger("click");
      expect(wrapper.emitted("save")).toEqual([[100]]);
    });

    it("offers “Mark as read” at 100 % and finishes in one PATCH, with the moment and no save", async () => {
      const wrapper = mountEditor({ value: 50, pages: 300, shelfId: 30, clubSlug: "readers" });
      await wrapper.get(".progress-trigger").trigger("click");
      expect(buttonText(wrapper, "Mark as read")).toBeUndefined();
      await wrapper.get(".sheet input[type=number]").setValue("300");
      expect(wrapper.get(".sheet .btn-primary").text()).toBe("Mark as read");
      expect(buttonText(wrapper, "Just save progress")).toBeDefined();
      const item = shelfItem({ id: 30, status: "finished" });
      patchShelf.mockResolvedValue(item);
      await buttonText(wrapper, "Mark as read")!.trigger("click");
      await flushPromises();
      expect(patchShelf).toHaveBeenCalledTimes(1);
      expect(patchShelf).toHaveBeenCalledWith(30, { status: "finished", position: 0 });
      expect(wrapper.emitted("finished")).toEqual([[item]]);
      expect(wrapper.emitted("save")).toBeUndefined();
      expect(wrapper.find(".sheet").exists()).toBe(false);
      expect(useFinish().item).toStrictEqual(item);
      expect(useFinish().clubSlug).toBe("readers");
    });

    it("can still save the progress alone", async () => {
      const wrapper = mountEditor({ value: 90, shelfId: 30 });
      await wrapper.get(".progress-trigger").trigger("click");
      await wrapper.get('.sheet button[aria-label="Forward 5 percent"]').trigger("click");
      await wrapper.get('.sheet button[aria-label="Forward 5 percent"]').trigger("click");
      await buttonText(wrapper, "Just save progress")!.trigger("click");
      expect(wrapper.emitted("save")).toEqual([[100]]);
      expect(patchShelf).not.toHaveBeenCalled();
      expect(useFinish().item).toBeNull();
    });

    it("offers it for a book already at 100 %, where only the progress save is disabled", async () => {
      const wrapper = mountEditor({ value: 100, shelfId: 30 });
      await wrapper.get(".progress-trigger").trigger("click");
      expect(buttonText(wrapper, "Mark as read")!.attributes("disabled")).toBeUndefined();
      expect(buttonText(wrapper, "Just save progress")!.attributes("disabled")).toBeDefined();
    });

    it("stays open with the error when the finish fails", async () => {
      const wrapper = mountEditor({ value: 100, shelfId: 30 });
      await wrapper.get(".progress-trigger").trigger("click");
      patchShelf.mockRejectedValue(new ApiError("Book not on your shelf", 404, "Book not on your shelf"));
      await buttonText(wrapper, "Mark as read")!.trigger("click");
      await flushPromises();
      expect(wrapper.get(".sheet [role=alert]").text()).toBe("Book not on your shelf");
      expect(wrapper.emitted("finished")).toBeUndefined();
      expect(useFinish().item).toBeNull();
    });
  });
});
