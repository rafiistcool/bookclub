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
    expect(wrapper.text()).toContain("No favourites yet");
    expect(wrapper.find(".portrait").exists()).toBe(false);
  });

  it("renders favourite covers under the profile header", async () => {
    const wrapper = await mountShelf(
      [favorite(1, "Circe"), favorite(2, "Galatea")],
      [item(9, "Something else")],
    );
    expect(wrapper.text()).not.toContain("No favourites yet");
    const portrait = wrapper.get("ul.portrait");
    expect(portrait.findAll("li a")).toHaveLength(2);
    expect(portrait.get("li a").attributes("role")).toBeUndefined();
    expect(portrait.text()).toContain("1");
    expect(portrait.text()).toContain("2");
  });
  it("offers mobile organization and rolls back failed moves while blocking overlapping moves", async () => {
    const first = item(1, "First"), second = { ...item(2, "Second"), position: 1 };
    const wrapper = await mountShelf([], [first, second]);
    expect(wrapper.find(".drag-handle").exists()).toBe(false);
    await wrapper.get(".organize-button").trigger("click");
    expect(wrapper.findAll(".drag-handle")).toHaveLength(2);
    let reject!: (reason: Error) => void;
    patchShelf.mockReturnValue(new Promise((_resolve, no) => { reject = no; }));
    const board = wrapper.getComponent(ShelfBoard);
    board.vm.$emit("dropped", second, "want_to_read", 0);
    await flushPromises();
    expect(board.props("disabled")).toBe(true);
    expect(wrapper.findAll(".book-tile-title").map((node) => node.text())).toEqual(["Second", "First"]);
    board.vm.$emit("dropped", first, "finished", 0);
    expect(patchShelf).toHaveBeenCalledTimes(1);
    reject(new Error("offline"));
    await flushPromises();
    expect(board.props("disabled")).toBe(false);
    expect(wrapper.findAll(".book-tile-title").map((node) => node.text())).toEqual(["First", "Second"]);
    wrapper.unmount();
  });

  it("edits reading dates and moves the book to the matching month", async () => {
    const book = { ...item(1, "Circe"), status: "finished" as const, finished_at: "2026-07-10T12:00:00Z" };
    const wrapper = await mountShelf([], [book]);
    expect(wrapper.text()).toContain("July 2026");
    await wrapper.get(".card-actions > button").trigger("click");
    await wrapper.findAll(".card-menu-content button").find((button) => button.text() === "Change reading date")!.trigger("click");
    await wrapper.get('input[type="date"]').setValue("2026-06-15");
    patchShelf.mockResolvedValue({ ...book, finished_at: "2026-06-15T12:00:00Z" });
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(1, { finished_on: "2026-06-15" });
    expect(wrapper.text()).toContain("June 2026");
    expect(wrapper.text()).not.toContain("July 2026");
    wrapper.unmount();
  });

});
