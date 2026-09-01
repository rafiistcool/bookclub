import { defineStore } from "pinia";
import { api } from "../api/client";
import type { BookRef, NextUpVote, VoteSuggestion } from "../types";
import { usePick } from "./pick";
import { swrLoad, swrState } from "./swr";

export const useVote = defineStore("vote", {
  state: () => ({
    vote: swrState<NextUpVote>(),
    suggestions: swrState<VoteSuggestion[]>(),
  }),
  getters: {
    data: (state) => state.vote.data,
    error: (state) => state.vote.error,
  },
  actions: {
    load(force = false) {
      return swrLoad(this.vote, () => api.nextUp(), force, "Could not load next up");
    },
    loadSuggestions(force = false) {
      return swrLoad(
        this.suggestions,
        async () => (await api.voteSuggestions()).items,
        force,
        "Could not load suggestions",
      );
    },
    _accept(next: NextUpVote) {
      this.vote.data = next;
      this.vote.loadedAt = Date.now();
      this.suggestions.loadedAt = 0;
    },
    async nominate(book: BookRef) {
      this._accept(await api.nominate(book));
    },
    async cast(nominationId: number) {
      this._accept(await api.castVote(nominationId));
    },
    async setDeadline(closesAt: string | null) {
      this._accept(await api.setVoteDeadline(closesAt));
    },
    async apply(nominationId: number, meetingAt: string | null) {
      const result = await api.applyWinner(nominationId, meetingAt);
      this._accept(result.vote);
      const pick = usePick();
      pick.current.data = { pick: result.pick, timezone: pick.timezone };
      pick.current.loadedAt = Date.now();
      pick.history.loadedAt = 0;
      return result;
    },
    invalidate() {
      this.vote.loadedAt = 0;
      this.suggestions.loadedAt = 0;
    },
  },
});
