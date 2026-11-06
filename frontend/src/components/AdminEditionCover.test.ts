import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { edition } from "../test/fixtures";

const editions = vi.fn();
const adminSetCover = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    editions: (...args: unknown[]) => editions(...args),
    adminSetCover: (...args: unknown[]) => adminSetCover(...args),
  },
}));

import { i18n } from "../i18n";
import AdminEditionCover from "./AdminEditionCover.vue";

function mountSheet() {
  return mount(AdminEditionCover, { props: { workId: 4, editionId: 12, title: "Circe" } });
}

describe("AdminEditionCover", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    editions.mockReset();
    adminSetCover.mockReset();
    i18n.global.locale.value = "en";
  });

  it("is a sheet of the work's editions as soon as it is mounted, the viewer's own marked", async () => {
    editions.mockResolvedValue({
      items: [
        edition({ id: 12, work_id: 4, format: "paperback", publisher: "Diogenes" }),
        edition({ id: 13, work_id: 4, format: "hardcover", publisher: "Hanser" }),
      ],
    });
    const wrapper = mountSheet();
    // No trigger of its own: the page's "…" sheet opens it.
    expect(wrapper.find(".sheet").attributes("aria-label")).toBe("An edition's cover");
    expect(wrapper.find(".editions-skeleton").exists()).toBe(true);
    await flushPromises();
    expect(editions).toHaveBeenCalledWith(4);
    const rows = wrapper.findAll(".edition-cover-row");
    expect(rows.map((row) => row.text().includes("Your edition"))).toEqual([true, false]);
    expect(rows[1]!.get("label.cover-photo").text()).toBe("Upload");
  });

  it("hands a chosen picture to the page with its edition, and uploads nothing itself", async () => {
    editions.mockResolvedValue({ items: [edition({ id: 13, work_id: 4 })] });
    const wrapper = mountSheet();
    await flushPromises();
    const input = wrapper.get(".edition-cover-row input[type=file]");
    const picture = new File(["x"], "cover.png", { type: "image/png" });
    Object.defineProperty(input.element, "files", { value: [picture], configurable: true });
    await input.trigger("change");
    expect(wrapper.emitted("picked")).toEqual([[13, picture]]);
    expect(adminSetCover).not.toHaveBeenCalled();
  });

  it("says so when the editions do not load, and closes on request", async () => {
    editions.mockRejectedValue(new TypeError("Failed to fetch"));
    const wrapper = mountSheet();
    await flushPromises();
    expect(wrapper.get(".error").text()).toBe(i18n.global.t("editions.loadFailed"));
    await wrapper.get(".sheet-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("says when the book has no editions", async () => {
    editions.mockResolvedValue({ items: [] });
    const wrapper = mountSheet();
    await flushPromises();
    expect(wrapper.text()).toContain("This book has no editions yet.");
  });
});
