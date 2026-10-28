import { afterEach, describe, expect, it, vi } from "vitest";
import { importFinished, importOutcome, importPercent, pollImport, skipLine } from "./goodreads";
import { i18n } from "./i18n";
import { importJob } from "./test/fixtures";

describe("Goodreads import jobs", () => {
  afterEach(() => {
    vi.useRealTimers();
    i18n.global.locale.value = "en";
  });

  it("reports progress until the job ends", () => {
    expect(importPercent(importJob({ status: "queued", total: 0 }))).toBe(0);
    expect(importPercent(importJob({ status: "running", total: 8, processed: 2 }))).toBe(25);
    expect(importPercent(importJob({ status: "done", total: 8, processed: 8 }))).toBe(100);
    expect(importFinished(importJob({ status: "running" }))).toBe(false);
    expect(importFinished(importJob({ status: "failed" }))).toBe(true);
  });

  it("translates each skip by its code and names untitled rows", () => {
    i18n.global.locale.value = "de";
    expect(skipLine({ title: "Circe", code: "no_match", reason: "No match" })).toBe("Circe — Kein Treffer im Katalog");
    expect(skipLine({ title: "", code: "no_title", reason: "No title" })).toBe("Ohne Titel — Kein Titel");
    i18n.global.locale.value = "en";
    expect(skipLine({ title: "Dune", code: "already_on_shelf", reason: "Already on your shelf" })).toBe(
      "Dune — Already on your shelf",
    );
  });

  it("sums up a finished job, and explains a failed one", () => {
    expect(importOutcome(importJob({ status: "done", imported: 3, skipped: 1 }))).toBe("Imported 3. Skipped 1.");
    expect(importOutcome(importJob({ status: "done", imported: 2 }))).toBe("Imported 2 books");
    expect(importOutcome(importJob({ status: "done", skipped: 4 }))).toBe("Skipped 4");
    expect(importOutcome(importJob({ status: "done" }))).toBe("Nothing to import");
    i18n.global.locale.value = "de";
    expect(
      importOutcome(importJob({ status: "failed", error: "The import stopped unexpectedly. Try again." })),
    ).toBe("Der Import ist unerwartet abgebrochen. Versuch es nochmal.");
  });

  it("polls every interval until the job is done", async () => {
    vi.useFakeTimers();
    const answers = [
      importJob({ status: "running", total: 3, processed: 1 }),
      importJob({ status: "running", total: 3, processed: 2 }),
      importJob({ status: "done", total: 3, processed: 3, imported: 3 }),
    ];
    const fetchJob = vi.fn(() => Promise.resolve(answers.shift()!));
    const seen: number[] = [];
    const done = pollImport(fetchJob, (job) => seen.push(job.processed), { interval: 2000 });
    await vi.advanceTimersByTimeAsync(1999);
    expect(fetchJob).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchJob).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4000);
    await expect(done).resolves.toMatchObject({ status: "done", imported: 3 });
    expect(seen).toEqual([1, 2, 3]);
  });

  it("stops when the page goes away, and gives up after repeated errors", async () => {
    vi.useFakeTimers();
    const controller = new AbortController();
    const fetchJob = vi.fn(() => Promise.resolve(importJob({ status: "running" })));
    const stopped = pollImport(fetchJob, () => {}, { interval: 100, signal: controller.signal });
    await vi.advanceTimersByTimeAsync(250);
    controller.abort();
    await expect(stopped).resolves.toBeNull();
    const calls = fetchJob.mock.calls.length;
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchJob).toHaveBeenCalledTimes(calls);

    const failing = vi.fn(() => Promise.reject(new Error("offline")));
    const gaveUp = pollImport(failing, () => {}, { interval: 100, maxErrors: 3 });
    const outcome = expect(gaveUp).rejects.toThrow("offline");
    await vi.advanceTimersByTimeAsync(300);
    await outcome;
    expect(failing).toHaveBeenCalledTimes(3);
  });
});
