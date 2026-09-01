import { defineStore } from "pinia";
import { api } from "../api/client";
import type { ClubPick, ClubPickBook, ClubPickCurrent } from "../types";
import { swrLoad, swrState } from "./swr";

export const usePick = defineStore("pick", {
  state: () => ({
    current: swrState<ClubPickCurrent>(),
    history: swrState<ClubPick[]>(),
  }),
  getters: {
    pick: (state) => state.current.data?.pick ?? null,
    timezone: (state) => state.current.data?.timezone ?? "UTC",
    loaded: (state) => state.current.data !== null,
    error: (state) => state.current.error,
  },
  actions: {
    load(force = false) {
      return swrLoad(this.current, () => api.clubPick(), force, "Could not load the club pick");
    },
    loadHistory(force = false) {
      return swrLoad(
        this.history,
        async () => (await api.clubPickHistory()).items,
        force,
        "Could not load past picks",
      );
    },
    async set(body: ClubPickBook) {
      const pick = await api.setClubPick(body);
      this.current.data = { pick, timezone: this.timezone };
      this.current.loadedAt = Date.now();
      this.history.loadedAt = 0;
      return pick;
    },
    async clear() {
      const result = await api.clearClubPick();
      this.current.data = result;
      this.current.loadedAt = Date.now();
      this.history.loadedAt = 0;
    },
    invalidate() {
      this.current.loadedAt = 0;
      this.history.loadedAt = 0;
    },
    /** Patch the viewer's shelf state on the cached pick after a shelf mutation. */
    markOnShelf(olWorkKey: string, status: ClubPick["on_shelf"], shelfId: number | null) {
      const pick = this.current.data?.pick;
      if (pick && pick.book.ol_work_key === olWorkKey) {
        pick.on_shelf = status;
        pick.shelf_id = shelfId;
      }
      this.current.loadedAt = 0;
    },
  },
});
