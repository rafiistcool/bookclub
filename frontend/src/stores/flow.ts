import { defineStore } from "pinia";
import { ApiError } from "../api/client";
import { STATUS_LABEL, type FinishNote, type Status } from "../constants";
import type { BookRef, ShelfItem } from "../types";
import { usePick } from "./pick";
import { useShelf } from "./shelf";
import { useToast } from "./toast";
import { useVote } from "./vote";

/**
 * Every book interaction in the app goes through one of these sheets. Pages
 * open a flow; <BookFlow> renders it; mutations land in the stores.
 */
export type Flow =
  | { kind: "details"; book: BookRef }
  | { kind: "status"; book: BookRef; item: ShelfItem | null; title?: string }
  | {
      kind: "finish";
      book: BookRef;
      item: ShelfItem | null;
      status: Extract<Status, "finished" | "did_not_finish">;
      position: number;
    }
  | { kind: "progress"; item: ShelfItem }
  | { kind: "clubPick"; book: BookRef }
  | { kind: "actions"; item: ShelfItem }
  | { kind: "remove"; item: ShelfItem }
  | { kind: "quote"; book: BookRef }
  | { kind: "scan" };

export function toRef(source: BookRef | ShelfItem["book"]): BookRef {
  return {
    ol_work_key: source.ol_work_key,
    title: source.title,
    authors: source.authors,
    cover_id: source.cover_id,
    year: source.year,
  };
}

export const useFlow = defineStore("flow", {
  state: () => ({
    stack: [] as Flow[],
  }),
  getters: {
    current: (state) => state.stack[state.stack.length - 1] ?? null,
  },
  actions: {
    open(flow: Flow) {
      this.stack = [flow];
    },
    push(flow: Flow) {
      this.stack.push(flow);
    },
    close() {
      this.stack.pop();
    },
    closeAll() {
      this.stack = [];
    },

    /** Add or move: existing shelf item wins; `status` may be null to prompt. */
    async chooseStatus(book: BookRef, item: ShelfItem | null, status: Status, position = 0) {
      if (status === "finished" || status === "did_not_finish") {
        this.push({ kind: "finish", book, item, status, position });
        return;
      }
      await this.applyStatus(book, item, status, position);
    },

    async applyStatus(
      book: BookRef,
      item: ShelfItem | null,
      status: Status,
      position = 0,
      note: FinishNote = {},
    ) {
      const shelf = useShelf();
      const toast = useToast();
      this.closeAll();
      const existing = item ?? shelf.byKey.get(book.ol_work_key) ?? null;
      try {
        if (existing) {
          const previous = { ...existing };
          if (previous.status === status && !Object.keys(note).length) {
            toast.show(`Already in ${STATUS_LABEL[status]}`);
            return;
          }
          await shelf.move(existing, status, position, note);
          toast.show(`Moved to ${STATUS_LABEL[status]}`, {
            label: "Undo",
            run: async () => {
              const live = shelf.items.find((row) => row.id === previous.id);
              if (!live) return;
              await shelf.move(live, previous.status, previous.position, {
                rating: previous.rating,
                take: previous.take,
                dnf_reason: previous.dnf_reason,
                progress: previous.progress,
              });
            },
          });
        } else {
          const created = await shelf.add(book, status, note);
          toast.show(`Added to ${STATUS_LABEL[status]}`, {
            label: "Undo",
            run: async () => {
              const live = shelf.items.find((row) => row.id === created.id);
              if (live) await shelf.remove(live);
            },
          });
        }
      } catch (err) {
        if (err instanceof ApiError && err.status === 409 && err.item) {
          await shelf.refresh();
          this.open({ kind: "status", book, item: err.item, title: "Already on your shelf — move it?" });
          return;
        }
        toast.show(err instanceof ApiError ? err.message : "Could not update your shelf");
      }
    },

    async setProgress(item: ShelfItem, progress: number | null) {
      const shelf = useShelf();
      const toast = useToast();
      this.closeAll();
      try {
        await shelf.setProgress(item, progress);
        toast.show(progress == null ? "Progress cleared" : `Progress: ${progress}%`);
      } catch (err) {
        toast.show(err instanceof ApiError ? err.message : "Could not save progress");
      }
    },

    async remove(item: ShelfItem) {
      const shelf = useShelf();
      const toast = useToast();
      this.closeAll();
      const snapshot = { ...item, book: { ...item.book } };
      try {
        await shelf.remove(item);
        toast.show(`Removed “${item.book.title}”`, {
          label: "Undo",
          run: async () => {
            await shelf.restore(snapshot);
          },
        });
      } catch (err) {
        toast.show(err instanceof ApiError ? err.message : "Could not remove that book");
      }
    },

    async setClubPick(book: BookRef, meetingAt: string | null, note: string) {
      const pick = usePick();
      const toast = useToast();
      this.closeAll();
      try {
        await pick.set({ ...book, meeting_at: meetingAt, note });
        useVote().invalidate();
        toast.show(`“${book.title}” is the club pick`);
      } catch (err) {
        toast.show(err instanceof ApiError ? err.message : "Could not set the club pick");
      }
    },

    async nominate(book: BookRef) {
      const vote = useVote();
      const toast = useToast();
      this.closeAll();
      try {
        await vote.nominate(book);
        toast.show("Nominated for next up");
      } catch (err) {
        toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
      }
    },
  },
});
