import { defineStore } from "pinia";
import { api } from "../api/client";
import type { FinishNote, Status } from "../constants";
import type { BookRef, ShelfItem } from "../types";
import { usePick } from "./pick";
import { swrLoad, swrState } from "./swr";

export const useShelf = defineStore("shelf", {
  state: () => ({
    shelf: swrState<ShelfItem[]>(),
  }),
  getters: {
    items: (state) => state.shelf.data ?? [],
    loaded: (state) => state.shelf.data !== null,
    error: (state) => state.shelf.error,
    byKey: (state) => {
      const map = new Map<string, ShelfItem>();
      for (const item of state.shelf.data ?? []) map.set(item.book.ol_work_key, item);
      return map;
    },
  },
  actions: {
    load(force = false) {
      return swrLoad(
        this.shelf,
        async () => (await api.myShelf()).items,
        force,
        "Could not load your shelf",
      );
    },
    _replace(items: ShelfItem[]) {
      this.shelf.data = items;
      this.shelf.loadedAt = Date.now();
    },
    async refresh() {
      this._replace((await api.myShelf()).items);
    },
    async add(book: BookRef, status: Status, note: FinishNote = {}) {
      const created = await api.addToShelf({ ...book, status, ...note });
      await this.refresh();
      usePick().markOnShelf(book.ol_work_key, created.status, created.id);
      return created;
    },
    async move(item: ShelfItem, status: Status, position = 0, note: FinishNote = {}) {
      const updated = await api.patchShelf(item.id, { status, position, ...note });
      await this.refresh();
      usePick().markOnShelf(item.book.ol_work_key, updated.status, updated.id);
      return updated;
    },
    async setProgress(item: ShelfItem, progress: number | null) {
      const updated = await api.patchShelf(item.id, { progress });
      await this.refresh();
      usePick().invalidate();
      return updated;
    },
    async remove(item: ShelfItem) {
      await api.removeFromShelf(item.id);
      this._replace(this.items.filter((row) => row.id !== item.id));
      usePick().markOnShelf(item.book.ol_work_key, null, null);
    },
    /** Re-add a removed item with its former status and notes (undo). */
    async restore(item: ShelfItem) {
      const note: FinishNote = {
        rating: item.rating,
        take: item.take,
        dnf_reason: item.dnf_reason,
        progress: item.progress,
      };
      const book: BookRef = {
        ol_work_key: item.book.ol_work_key,
        title: item.book.title,
        authors: item.book.authors,
        cover_id: item.book.cover_id,
        year: item.book.year,
      };
      return this.add(book, item.status, note);
    },
    invalidate() {
      this.shelf.loadedAt = 0;
    },
  },
});
