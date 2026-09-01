import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ShelfItem } from "../types";

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    api: {
      myShelf: vi.fn(),
      addToShelf: vi.fn(),
      patchShelf: vi.fn(),
      removeFromShelf: vi.fn(),
      clubPick: vi.fn().mockResolvedValue({ pick: null, timezone: "UTC" }),
      setClubPick: vi.fn(),
      nominate: vi.fn(),
    },
  };
});

import { api } from "../api/client";
import { useFlow } from "./flow";
import { useShelf } from "./shelf";
import { useToast } from "./toast";

const BOOK = { ol_work_key: "/works/OL1W", title: "Circe", authors: "Madeline Miller", cover_id: 1, year: 2018 };

function item(overrides: Partial<ShelfItem> = {}): ShelfItem {
  return {
    id: 1,
    status: "want_to_read",
    position: 0,
    updated_at: "2026-01-01T00:00:00Z",
    started_at: null,
    finished_at: null,
    book: { id: 1, cover_url: null, ...BOOK },
    rating: null,
    take: "",
    dnf_reason: "",
    progress: null,
    ...overrides,
  };
}

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

describe("flow store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    mocked.myShelf.mockResolvedValue({ user: { id: 1, username: "ada" }, items: [] });
  });

  it("opens the finish sheet instead of applying finished directly", async () => {
    const flow = useFlow();
    await flow.chooseStatus(BOOK, null, "finished");
    expect(flow.current).toMatchObject({ kind: "finish", status: "finished" });
    expect(mocked.addToShelf).not.toHaveBeenCalled();
  });

  it("adds a new book and offers undo that removes it", async () => {
    const created = item({ id: 42, status: "want_to_read" });
    mocked.addToShelf.mockResolvedValue(created);
    mocked.myShelf.mockResolvedValue({ user: { id: 1, username: "ada" }, items: [created] });
    const flow = useFlow();
    const toast = useToast();
    flow.open({ kind: "status", book: BOOK, item: null });

    await flow.applyStatus(BOOK, null, "want_to_read");

    expect(mocked.addToShelf).toHaveBeenCalledWith({ ...BOOK, status: "want_to_read" });
    expect(flow.current).toBeNull();
    expect(toast.message).toBe("Added to Want to read");
    expect(toast.action?.label).toBe("Undo");

    mocked.removeFromShelf.mockResolvedValue(undefined);
    await toast.act();
    expect(mocked.removeFromShelf).toHaveBeenCalledWith(42);
    expect(useShelf().items).toEqual([]);
  });

  it("moves an existing item and undo restores the previous status and note", async () => {
    const before = item({ id: 7, status: "finished", rating: 4, take: "Great" });
    const after = item({ id: 7, status: "currently_reading", rating: null, take: "" });
    mocked.patchShelf.mockResolvedValue(after);
    mocked.myShelf.mockResolvedValue({ user: { id: 1, username: "ada" }, items: [after] });
    const shelf = useShelf();
    shelf.shelf.data = [before];
    shelf.shelf.loadedAt = Date.now();
    const flow = useFlow();
    const toast = useToast();

    await flow.applyStatus(BOOK, before, "currently_reading", 0);
    expect(mocked.patchShelf).toHaveBeenCalledWith(7, { status: "currently_reading", position: 0 });
    expect(toast.message).toBe("Moved to Reading");

    mocked.patchShelf.mockClear();
    await toast.act();
    expect(mocked.patchShelf).toHaveBeenCalledWith(7, {
      status: "finished",
      position: 0,
      rating: 4,
      take: "Great",
      dnf_reason: "",
      progress: null,
    });
  });

  it("removes with undo that re-adds the item with its notes", async () => {
    const gone = item({ id: 3, status: "finished", rating: 5, take: "Yes" });
    const shelf = useShelf();
    shelf.shelf.data = [gone];
    shelf.shelf.loadedAt = Date.now();
    mocked.removeFromShelf.mockResolvedValue(undefined);
    mocked.addToShelf.mockResolvedValue(gone);
    const flow = useFlow();
    const toast = useToast();

    await flow.remove(gone);
    expect(mocked.removeFromShelf).toHaveBeenCalledWith(3);
    expect(shelf.items).toEqual([]);
    expect(toast.message).toContain("Removed");

    await toast.act();
    expect(mocked.addToShelf).toHaveBeenCalledWith({
      ...BOOK,
      status: "finished",
      rating: 5,
      take: "Yes",
      dnf_reason: "",
      progress: null,
    });
  });

  it("turns a 409 into a move prompt for the existing item", async () => {
    const { ApiError } = await vi.importActual<typeof import("../api/client")>("../api/client");
    const existing = item({ id: 11, status: "finished" });
    mocked.addToShelf.mockRejectedValue(new ApiError("Already on your shelf", 409, null, existing));
    const flow = useFlow();
    await flow.applyStatus(BOOK, null, "want_to_read");
    expect(flow.current).toMatchObject({ kind: "status", item: { id: 11 }, title: "Already on your shelf — move it?" });
  });

  it("reports an unchanged status without a network call", async () => {
    const same = item({ id: 5, status: "currently_reading" });
    const flow = useFlow();
    const toast = useToast();
    await flow.applyStatus(BOOK, same, "currently_reading");
    expect(mocked.patchShelf).not.toHaveBeenCalled();
    expect(toast.message).toBe("Already in Reading");
  });
});
