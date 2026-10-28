import { t } from "./i18n";
import { translateApiError } from "./i18n/apiErrors";
import type { GoodreadsImport, GoodreadsSkip } from "./types";

/** How often a running import is asked for its progress. */
export const IMPORT_POLL_MS = 2000;
/** Remembers the last import of this tab, so coming back to Settings shows it again. */
export const IMPORT_JOB_KEY = "bookclub.import-job";

export function importFinished(job: GoodreadsImport): boolean {
  return job.status === "done" || job.status === "failed";
}

/** Rows looked up so far, 0–100. */
export function importPercent(job: GoodreadsImport): number {
  if (importFinished(job)) return 100;
  if (!job.total) return 0;
  return Math.min(100, Math.round((job.processed / job.total) * 100));
}

/**
 * The skip in the reader's language: translated by code, with the server's
 * English as a fallback. A row without a title comes as `"Untitled"`
 * (`no_title`), which is translated too; `stopped_after_400` names no row.
 */
export function skipLine(skip: GoodreadsSkip): string {
  const key = `settings.importSkip.${skip.code}`;
  const translated = t(key);
  const reason = translated === key ? skip.reason : translated;
  const title = skip.code === "no_title" ? t("settings.untitled") : skip.title;
  return title ? t("settings.importSkipLine", { title, reason }) : reason;
}

/** What a finished import says in its toast. */
export function importOutcome(job: GoodreadsImport): string {
  if (job.status === "failed") {
    return job.error ? translateApiError(job.error) : t("settings.importFailed");
  }
  if (job.imported) {
    return job.skipped
      ? t("settings.importedSkipped", { imported: job.imported, skipped: job.skipped })
      : t("settings.importedBooks", { n: job.imported });
  }
  return job.skipped ? t("settings.skippedOnly", { n: job.skipped }) : t("settings.nothingToImport");
}

/**
 * Ask for the job every `interval` ms until it is done or failed, reporting
 * each answer. Stops early, without an answer, when `signal` aborts. A failed
 * request is retried on the next tick; `maxErrors` in a row give up.
 */
export async function pollImport(
  fetchJob: () => Promise<GoodreadsImport>,
  onUpdate: (job: GoodreadsImport) => void,
  { interval = IMPORT_POLL_MS, signal, maxErrors = 5 }: { interval?: number; signal?: AbortSignal; maxErrors?: number } = {},
): Promise<GoodreadsImport | null> {
  let errors = 0;
  for (;;) {
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, interval);
      signal?.addEventListener("abort", () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
    });
    if (signal?.aborted) return null;
    try {
      const job = await fetchJob();
      errors = 0;
      if (signal?.aborted) return null;
      onUpdate(job);
      if (importFinished(job)) return job;
    } catch (err) {
      errors += 1;
      if (errors >= maxErrors) throw err;
    }
  }
}
