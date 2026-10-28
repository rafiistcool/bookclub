// A report of a picture that was already taken down (#54, work-plan ADM scope 2: answered
// idempotently with the removed report). It tests components/ReportCoverSheet.vue, which ADM
// owns, but lives here because its colocated test file components/ReportCoverSheet.test.ts is
// outside ADM's files (docs/phase2/notes/adm.md).
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

const reportCover = vi.fn();

vi.mock("../../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { reportCover: (...args: unknown[]) => reportCover(...args) },
}));

import { useToast } from "../../stores/toast";
import ReportCoverSheet from "../ReportCoverSheet.vue";

describe("ReportCoverSheet: a picture already taken down", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    reportCover.mockReset();
  });

  it("thanks the reader for an open report", async () => {
    reportCover.mockResolvedValue({ id: 1, status: "open" });
    const wrapper = mount(ReportCoverSheet, { props: { publicId: "abc2def", title: "Circe" } });
    await wrapper.get('input[value="offensive"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(useToast().items[0]?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("says so when the picture was already taken down", async () => {
    reportCover.mockResolvedValue({ id: 4, status: "removed" });
    const wrapper = mount(ReportCoverSheet, { props: { publicId: "abc2def", title: "Circe" } });
    await wrapper.get('input[value="offensive"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(reportCover).toHaveBeenCalledWith("abc2def", { reason: "offensive", details: null });
    expect(useToast().items[0]?.message).toBe("This picture was already taken down. Thanks anyway.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });
});
