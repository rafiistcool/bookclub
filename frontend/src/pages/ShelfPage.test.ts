import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Favorite, ShelfItem } from "../types";

const myShelf = vi.fn();
const clubPick = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    myShelf: (...args: unknown[]) => myShelf(...args),
    clubPick: (...args: unknown[]) => clubPick(...args),
  },
}));

import { useSession } from "../stores/session";
import ShelfPage from "./ShelfPage.vue";

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
});
