import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Favorite, ShelfItem } from "../types";

const myShelf = vi.fn();
const clubPick = vi.fn();
const patchShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    patchShelf: (...args: unknown[]) => patchShelf(...args),
    myShelf: (...args: unknown[]) => myShelf(...args),
    clubPick: (...args: unknown[]) => clubPick(...args),
  },
}));

import { useSession } from "../stores/session";
import ShelfPage from "./ShelfPage.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import { i18n } from "../i18n";

function favorite(position: number, title: string): Favorite {
  return {
    position,
    book: {
      id: position,
      ol_work_key: `/works/OL${position}W`,
      title,
      authors: "An Author",
      cover_id: position,
      year: 2018,
      cover_url: null,
    },
  };
}

function item(id: number, title: string): ShelfItem {
  return {
    id,
    status: "want_to_read",
    position: 0,
    updated_at: "2026-09-01T00:00:00Z",
    book: {
      id,
      ol_work_key: `/works/OL${id}W`,
      title,
      authors: "An Author",
      cover_id: id,
      year: 2018,
      cover_url: null,
    },
    rating: null,
    take: "",
    dnf_reason: "",
    progress: null,
  };
}

async function mountShelf(favorites: Favorite[], items: ShelfItem[] = []) {
  myShelf.mockResolvedValue({
    user: {
      id: 1,
      username: "ada",
      theme: "paper",
      color_mode: "system",
      locale: "en",
      avatar_url: null,
    },
    items,
    favorites,
  });
  clubPick.mockResolvedValue({ pick: null, timezone: "UTC" });
  const pinia = createPinia();
  setActivePinia(pinia);
  const session = useSession();
  session.user = {
    id: 1,
    username: "ada",
    theme: "paper",
    color_mode: "system",
    locale: "en",
    avatar_url: null,
  };
  session.ready = true;
  const wrapper = mount(ShelfPage, {
    global: { plugins: [pinia] },
  });
  await flushPromises();
  return wrapper;
}

describe("ShelfPage favourites portrait", () => {
  beforeEach(() => {
    myShelf.mockReset();
    clubPick.mockReset();
    patchShelf.mockReset();
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      configurable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
  });

  it("shows a subtle empty hint when you have no favourites", async () => {
    const wrapper = await mountShelf([]);
    await wrapper.get(".favorites-toggle").trigger("click");
    expect(wrapper.text()).toContain("No favourites yet");
    expect(wrapper.find(".portrait").exists()).toBe(false);
  });

  it("renders favourite covers under the profile header", async () => {
    const wrapper = await mountShelf(
      [favorite(1, "Circe"), favorite(2, "Galatea")],
      [item(9, "Something else")],
    );
    expect(wrapper.text()).not.toContain("No favourites yet");
    expect(wrapper.find("ul.portrait").exists()).toBe(false);
    await wrapper.get(".favorites-toggle").trigger("click");
    expect(wrapper.get(".favorites-toggle").attributes("aria-expanded")).toBe("true");
    const portrait = wrapper.get("ul.portrait");
    expect(portrait.findAll("li a")).toHaveLength(2);
    expect(portrait.get("li a").attributes("role")).toBeUndefined();
    expect(portrait.text()).toContain("1");
    expect(portrait.text()).toContain("2");
    await wrapper.get(".favorites-toggle").trigger("click");
    expect(wrapper.find("ul.portrait").exists()).toBe(false);
    expect(wrapper.get(".favorites-toggle").attributes("aria-expanded")).toBe("false");
  });
  it("allows direct reordering and rolls back failed saves while blocking overlapping moves", async () => {
    const first = item(1, "First"), second = { ...item(2, "Second"), position: 1 };
    const wrapper = await mountShelf([], [first, second]);
    expect(wrapper.find(".drag-handle").exists()).toBe(false);
    expect(wrapper.find(".organize-button").exists()).toBe(false);
    let reject!: (reason: Error) => void;
    patchShelf.mockReturnValue(new Promise((_resolve, no) => { reject = no; }));
    const board = wrapper.getComponent(ShelfBoard);
    board.vm.$emit("reordered", second, 0);
    await flushPromises();
    expect(board.props("disabled")).toBe(true);
    expect(wrapper.findAll(".book-tile-title").map((node) => node.text())).toEqual(["Second", "First"]);
    board.vm.$emit("reordered", first, 0);
    expect(patchShelf).toHaveBeenCalledTimes(1);
    expect(patchShelf).toHaveBeenCalledWith(2, { shelf_position: 0 });
    reject(new Error("offline"));
    await flushPromises();
    expect(board.props("disabled")).toBe(false);
    expect(wrapper.findAll(".book-tile-title").map((node) => node.text())).toEqual(["First", "Second"]);
    wrapper.unmount();
  });

  it("saves a cross-status reorder without changing status, then lets status be changed separately", async () => {
    const first = item(1, "Unread");
    const finished = { ...item(2, "Finished book"), status: "finished" as const, rating: 5,
      finished_at: "2026-07-10T12:00:00Z" };
    const wrapper = await mountShelf([], [first, finished]);
    patchShelf.mockResolvedValueOnce({ ...finished, shelf_position: 0 });
    const board = wrapper.getComponent(ShelfBoard);
    board.vm.$emit("reordered", finished, 0);
    await flushPromises();
    expect(patchShelf).toHaveBeenLastCalledWith(2, { shelf_position: 0 });
    expect(wrapper.findAll(".book-tile-title").map(node => node.text())).toEqual(["Unread", "Finished book"]);
    expect(board.props("items")[0]).toMatchObject({ status: "finished", rating: 5, finished_at: finished.finished_at });

    await wrapper.findAll("button.shelf-status")[0].trigger("click");
    patchShelf.mockResolvedValueOnce({ ...first, status: "currently_reading", shelf_position: 1 });
    await wrapper.findAll(".sheet button").find(button => button.text() === "Reading")!.trigger("click");
    await flushPromises();
    expect(patchShelf).toHaveBeenLastCalledWith(1, { status: "currently_reading", position: 0 });
    expect(wrapper.findAll(".book-tile-title").map(node => node.text())).toEqual(["Unread", "Finished book"]);
    wrapper.unmount();
  });

  it("keeps reading-date editing available from the visible status button", async () => {
    const book = { ...item(1, "Circe"), status: "finished" as const, finished_at: "2026-07-10T12:00:00Z" };
    const wrapper = await mountShelf([], [book]);
    await wrapper.get("button.shelf-status").trigger("click");
    await wrapper.findAll("button").find((button) => button.text() === "Change reading date")!.trigger("click");
    await wrapper.get("select#date-precision").setValue("day");
    await wrapper.get('input[type="date"]').setValue("2026-06-15");
    patchShelf.mockResolvedValue({ ...book, finished_at: "2026-06-15T12:00:00Z" });
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(1, { finished_on: "2026-06-15" });
    expect(wrapper.getComponent(ShelfBoard).props("items")[0].finished_at).toBe("2026-06-15T12:00:00Z");
    expect(wrapper.get("time").text()).toBe("June 2026");
    wrapper.unmount();
  });


  it("saves a month/year without requiring a day", async () => {
    const book = { ...item(1, "Circe"), status: "finished" as const, finished_at: "2026-09-10T12:00:00Z" };
    const wrapper = await mountShelf([], [book]);
    await wrapper.get("button.shelf-status").trigger("click");
    await wrapper.findAll("button").find(button => button.text() === "Change reading date")!.trigger("click");
    expect(wrapper.get<HTMLInputElement>('input[type="month"]').element.value).toBe("2026-09");
    await wrapper.get('input[type="month"]').setValue("2025-12");
    patchShelf.mockResolvedValue({ ...book, finished_at: "2025-12-01T12:00:00Z" });
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(1, { finished_on: "2025-12-01" });
    expect(wrapper.text()).toContain("December 2025");
    wrapper.unmount();
  });

  it("saves cross-month order and completion together, and rolls both back on failure", async () => {
    const book = { ...item(1, "Circe"), status: "finished" as const, rating: 5, finished_at: "2026-09-10T12:00:00Z" };
    const wrapper = await mountShelf([], [book]);
    const board = wrapper.getComponent(ShelfBoard);
    patchShelf.mockRejectedValueOnce(new Error("offline"));
    board.vm.$emit("reordered", book, 0, "2026-08");
    await flushPromises();
    expect(patchShelf).toHaveBeenLastCalledWith(1, { shelf_position: 0, finished_on: "2026-08-01" });
    expect(board.props("items")[0]).toEqual(book);
    const saved = { ...book, shelf_position: 0, finished_at: "2026-08-01T12:00:00Z" };
    patchShelf.mockResolvedValueOnce(saved);
    board.vm.$emit("reordered", book, 0, "2026-08");
    await flushPromises();
    expect(board.props("items")[0]).toEqual(saved);
    expect(wrapper.get("time").text()).toBe("August 2026");
    patchShelf.mockResolvedValueOnce(saved);
    board.vm.$emit("reordered", saved, 0, "2026-08");
    await flushPromises();
    expect(patchShelf).toHaveBeenLastCalledWith(1, { shelf_position: 0 });
    wrapper.unmount();
  });

  it("renders shelf hints in both languages instead of showing translation keys", async () => {
    const wrapper = await mountShelf([], [item(1, "A book")]);
    const previous = i18n.global.locale.value;
    for (const locale of ["de", "en"] as const) {
      i18n.global.locale.value = locale;
      await flushPromises();
      expect(wrapper.get(".shelf-drag-hint").text()).toBe(i18n.global.messages.value[locale].shelf.chronologicalHint);
      expect(wrapper.text()).not.toContain("shelf.directDragHint");
    }
    i18n.global.locale.value = previous;
    wrapper.unmount();
  });

});
