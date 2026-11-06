import { defineStore } from "pinia";
import { computed, shallowRef } from "vue";
import type { ShelfPatchIn } from "../api/generated";
import { ancestry } from "../focusBack";
import type { ShelfItem, Visibility } from "../types";

/** What the moment is opened with. */
export type Celebration = {
  /** The shelf entry as the server returned it after the finish. */
  item: ShelfItem;
  /** The club whose current pick the book is, for the "2 of 4" line; none means no line and no club call. */
  clubSlug?: string | null;
  /**
   * What the moment gives focus back to when it closes, or the nearest of its
   * ancestors still on the page: the finish often removes the control that
   * opened it (a progress row, the hero's action). Default: what has focus
   * when `celebrate` is called.
   */
  opener?: Element | null;
};

interface Pending {
  /** Counts up with every moment, so a moment that replaces an open one mounts afresh. */
  serial: number;
  item: ShelfItem;
  clubSlug: string | null;
  /** The opener and its ancestors, noted while they were on the page. */
  returnFocus: HTMLElement[];
  resolve: (saved: ShelfItem | null) => void;
}

/**
 * Whether a write moved the book to finished just now. The moment opens only
 * then: a retry after a 409 whose entry was already finished, or a tap on the
 * current status, changed nothing and celebrates nothing.
 */
export function isFinishTransition(previous: string | null | undefined, item: Pick<ShelfItem, "status">): boolean {
  return previous !== "finished" && item.status === "finished";
}

/** The reader's choices in the moment. A null rating or take means "not set". */
export type MomentDraft = {
  rating: number | null;
  take: string;
  onlyMe: boolean;
};

/**
 * The one PATCH the moment's "Done" sends: only what the reader set and what
 * differs from the stored entry. Null when there is nothing to save. The
 * visibility travels with the rating, so an "only me" rating is sealed in
 * the feed from the start.
 */
export function momentPatch(item: ShelfItem, draft: MomentDraft): ShelfPatchIn | null {
  const body: ShelfPatchIn = {};
  if (draft.rating !== null && draft.rating !== item.rating) body.rating = draft.rating;
  const take = draft.take.trim();
  if (take && take !== item.take) body.take = take;
  const visibility: Visibility = draft.onlyMe ? "only_me" : "audience";
  if (visibility !== (item.rating_visibility ?? "audience")) body.rating_visibility = visibility;
  return Object.keys(body).length ? body : null;
}

/**
 * The finished moment's hook (`components/FinishedMoment.vue`, which App.vue
 * loads and mounts while one is open). Every path that finishes a book calls
 * `celebrate` after its own successful PATCH; it resolves with the entry as
 * the moment saved it, or null when the reader skipped (or another moment
 * took its place). `lastSaved` carries the newest saved entry, for a page
 * that did not open the moment itself (a ProgressEditor did) to adopt the
 * stars.
 */
export const useFinish = defineStore("finish", () => {
  const pending = shallowRef<Pending | null>(null);
  const item = computed(() => pending.value?.item ?? null);
  const clubSlug = computed(() => pending.value?.clubSlug ?? null);
  const serial = computed(() => pending.value?.serial ?? 0);
  const returnFocus = computed(() => pending.value?.returnFocus ?? []);
  const lastSaved = shallowRef<ShelfItem | null>(null);
  let opened = 0;

  function celebrate({ item: entry, clubSlug: slug = null, opener }: Celebration): Promise<ShelfItem | null> {
    if (entry.status !== "finished") return Promise.resolve(null);
    // Noted now: by the time the moment closes, the opener may be gone.
    const from = opener === undefined ? (typeof document === "undefined" ? null : document.activeElement) : opener;
    const returnFocus = typeof document === "undefined" ? [] : ancestry(from);
    // A newer finish wins; the older moment closes as skipped.
    pending.value?.resolve(null);
    return new Promise((resolve) => {
      opened += 1;
      pending.value = { serial: opened, item: entry, clubSlug: slug, returnFocus, resolve };
    });
  }

  /**
   * Called by the moment: with the saved entry after "Done", null after
   * "Skip". `which` is the serial of the moment closing: a save that returns
   * after a newer moment took its place does not close that one; its entry
   * still becomes `lastSaved`, so a page shows the stars it saved.
   */
  function close(saved: ShelfItem | null = null, which?: number) {
    const open = pending.value;
    if (saved) lastSaved.value = saved;
    if (!open || (which !== undefined && which !== open.serial)) return;
    pending.value = null;
    open.resolve(saved);
  }

  return { item, clubSlug, serial, returnFocus, lastSaved, celebrate, close };
});
