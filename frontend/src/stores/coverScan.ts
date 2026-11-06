import { defineStore } from "pinia";
import { computed, shallowRef } from "vue";

/** What a request is answered with: the flat scan, the photo as it was, or null when cancelled. */
export type CoverScanResult = File | null;

interface Pending {
  file: File;
  resolve: (result: CoverScanResult) => void;
  /** What had focus when the photo was requested, for the scan screen to give it back. */
  opener: HTMLElement | null;
}

/**
 * The cover scanner's hook (`components/CoverScanner.vue`, which App.vue
 * loads and mounts while a photo is open). `request(file)` opens the scan screen for a photo of a cover and
 * resolves with what the reader chose: the flattened cover, the original file
 * ("Whole photo"), or null when they cancelled. Only one photo is open at a
 * time; a second request while one is open resolves null at once.
 */
export const useCoverScan = defineStore("coverScan", () => {
  // Shallow: a File and a callback are not state to make deeply reactive.
  const pending = shallowRef<Pending | null>(null);
  const file = computed(() => pending.value?.file ?? null);
  const opener = computed(() => pending.value?.opener ?? null);

  function request(photo: File): Promise<CoverScanResult> {
    if (pending.value) return Promise.resolve(null);
    // Noted now, while the caller still holds focus: the scan screen loads
    // later, when a CoverPhotoButton has disabled (and so blurred) its input.
    const focused = typeof document === "undefined" ? null : document.activeElement;
    const opener = focused instanceof HTMLElement && focused !== document.body ? focused : null;
    return new Promise((resolve) => {
      pending.value = { file: photo, resolve, opener };
    });
  }

  /** Called by the scan screen with the reader's choice; closes it. */
  function finish(result: CoverScanResult) {
    const open = pending.value;
    if (!open) return;
    pending.value = null;
    open.resolve(result);
  }

  return { file, opener, request, finish };
});
