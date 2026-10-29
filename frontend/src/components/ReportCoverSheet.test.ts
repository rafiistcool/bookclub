import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

const reportCover = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { reportCover: (...args: unknown[]) => reportCover(...args) },
}));

import { ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import ReportCoverSheet from "./ReportCoverSheet.vue";

describe("ReportCoverSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    reportCover.mockReset();
  });

  it("sends the reason and details for the cover's media id", async () => {
    reportCover.mockResolvedValue({ id: 1 });
    const wrapper = mount(ReportCoverSheet, { props: { publicId: "abc2def", title: "Circe" } });
    const send = wrapper.get('button[type="submit"]');
    expect(send.attributes("disabled")).toBeDefined();
    await wrapper.get('input[value="wrong_book"]').setValue(true);
    await wrapper.get("textarea").setValue("  It shows Galatea.  ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(reportCover).toHaveBeenCalledWith("abc2def", { reason: "wrong_book", details: "It shows Galatea." });
    expect(useToast().items[0]?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("keeps the sheet open with the server's answer when the report fails", async () => {
    reportCover.mockRejectedValue(new ApiError("Slow down a little. Try again later.", 429, "Slow down a little. Try again later."));
    const wrapper = mount(ReportCoverSheet, { props: { publicId: "abc2def", title: "Circe" } });
    await wrapper.get('input[value="other"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(reportCover.mock.calls[0][1]).toEqual({ reason: "other", details: null });
    expect(wrapper.text()).toContain("Slow down a little. Try again later.");
    expect(wrapper.emitted("close")).toBeUndefined();
  });
});
