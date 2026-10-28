import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { importJob } from "../test/fixtures";

const importGoodreads = vi.fn();
const fetchJob = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    job?: unknown;
    constructor(message: string, status: number, _detail?: unknown, body?: { job?: unknown }) {
      super(message);
      this.status = status;
      this.job = body?.job;
    }
  },
  api: {
    importGoodreads: (...args: unknown[]) => importGoodreads(...args),
    importJob: (...args: unknown[]) => fetchJob(...args),
  },
}));

import { ApiError } from "../api/client";
import { IMPORT_JOB_KEY, IMPORT_POLL_MS } from "../goodreads";
import { i18n } from "../i18n";
import { useToast } from "../stores/toast";
import GoodreadsImport from "./GoodreadsImport.vue";

async function chooseAndStart(wrapper: ReturnType<typeof mount>) {
  const input = wrapper.get('input[type="file"]');
  Object.defineProperty(input.element, "files", { value: [new File(["Title"], "export.csv")] });
  await input.trigger("change");
  await wrapper.get("button").trigger("click");
  await flushPromises();
}

describe("GoodreadsImport", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    importGoodreads.mockReset();
    fetchJob.mockReset();
    sessionStorage.clear();
    i18n.global.locale.value = "en";
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("uploads the export, follows the job and lists translated skips", async () => {
    importGoodreads.mockResolvedValue(importJob({ id: 7, status: "queued", total: 3 }));
    fetchJob
      .mockResolvedValueOnce(importJob({ id: 7, status: "running", total: 3, processed: 1, imported: 1 }))
      .mockResolvedValueOnce(importJob({
        id: 7, status: "done", total: 3, processed: 3, imported: 2, skipped: 1,
        skips: [{ title: "", code: "no_match", reason: "No match" }],
      }));
    const wrapper = mount(GoodreadsImport, { global: { plugins: [i18n] } });
    await chooseAndStart(wrapper);
    expect(importGoodreads).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("Waiting to start…");
    expect(sessionStorage.getItem(IMPORT_JOB_KEY)).toBe("7");

    await vi.advanceTimersByTimeAsync(IMPORT_POLL_MS);
    expect(fetchJob).toHaveBeenCalledWith(7);
    expect(wrapper.text()).toContain("1 of 3 looked up");
    expect(wrapper.get('[role="progressbar"]').attributes("aria-valuenow")).toBe("33");

    await vi.advanceTimersByTimeAsync(IMPORT_POLL_MS);
    expect(wrapper.find('[role="progressbar"]').exists()).toBe(false);
    expect(wrapper.text()).toContain("Imported 2. Skipped 1.");
    expect(wrapper.get(".import-skips").text()).toBe("Untitled — No match in the catalog");
    expect(useToast().items.at(-1)?.message).toBe("Imported 2. Skipped 1.");
    wrapper.unmount();
  });

  it("picks up the import that is already running instead of starting another", async () => {
    const running = importJob({ id: 3, status: "running", total: 10, processed: 4 });
    importGoodreads.mockRejectedValue(
      new ApiError("An import is already running", 409, "An import is already running", { job: running }),
    );
    fetchJob.mockResolvedValue({ ...running, processed: 5 });
    const wrapper = mount(GoodreadsImport, { global: { plugins: [i18n] } });
    await chooseAndStart(wrapper);
    expect(wrapper.text()).toContain("4 of 10 looked up");
    await vi.advanceTimersByTimeAsync(IMPORT_POLL_MS);
    expect(fetchJob).toHaveBeenCalledWith(3);
    expect(wrapper.text()).toContain("5 of 10 looked up");
    wrapper.unmount();
  });

  it("shows this tab's import again after coming back to the page", async () => {
    sessionStorage.setItem(IMPORT_JOB_KEY, "9");
    fetchJob.mockResolvedValue(importJob({ id: 9, status: "failed", error: "The import stopped unexpectedly. Try again." }));
    const wrapper = mount(GoodreadsImport, { global: { plugins: [i18n] } });
    await flushPromises();
    expect(fetchJob).toHaveBeenCalledWith(9);
    expect(wrapper.get(".error").text()).toBe("The import stopped unexpectedly. Try again.");
    wrapper.unmount();
  });
});
