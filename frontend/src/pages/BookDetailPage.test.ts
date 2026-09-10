import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { BookDetail } from "../types";

const book = vi.fn();
const clubPick = vi.fn();
const addFavorite = vi.fn();
const removeFavorite = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.name = "ApiError";
      this.status = status;
    }
  },
  api: {
    book: (...args: unknown[]) => book(...args),
    clubPick: (...args: unknown[]) => clubPick(...args),
    addFavorite: (...args: unknown[]) => addFavorite(...args),
    removeFavorite: (...args: unknown[]) => removeFavorite(...args),
  },
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useToast } from "../stores/toast";
import BookDetailPage from "./BookDetailPage.vue";

function detail(overrides: Partial<BookDetail> = {}): BookDetail {
  return {
    id: 4,
    ol_work_key: "/works/BCcd3ae462c3",
    title: "The Witch of Portobello",
    authors: "Paulo Coelho",
    cover_id: null,
    cover_url: null,
    year: 2006,
    description: "",
    subjects: [],
    on_shelf: null,
    shelf_id: null,
    club_pick: false,
    rating: null,
    take: "",
    dnf_reason: "",
    progress: null,
    readers: [],
    club_rating: null,
    rating_count: 0,
    custom: true,
    favorite_position: null,
    ...overrides,
  };
}

async function mountBook() {
  book.mockResolvedValue(detail());
  clubPick.mockResolvedValue({ pick: null, timezone: "UTC" });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/book/:workId", component: BookDetailPage }],
  });
  await router.push("/book/BCcd3ae462c3");
  await router.isReady();
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(BookDetailPage, {
    global: { plugins: [pinia, router] },
  });
  await flushPromises();
  return wrapper;
}

describe("BookDetailPage favourite control", () => {
  beforeEach(() => {
    book.mockReset();
    clubPick.mockReset();
    addFavorite.mockReset();
    removeFavorite.mockReset();
    i18n.global.locale.value = "en";
  });

  it("adds a favourite when there is a free slot", async () => {
    addFavorite.mockResolvedValue({
      items: [{ position: 1, book: { id: 4, title: "The Witch of Portobello" } }],
    });
    const wrapper = await mountBook();
    const button = wrapper.findAll("button").find((node) => node.text().includes("Favourite"));
    expect(button).toBeTruthy();
    await button!.trigger("click");
    await flushPromises();
    expect(addFavorite).toHaveBeenCalledWith(4);
    expect(button!.text()).toContain("Favourite · 1");
    expect(button!.attributes("aria-pressed")).toBe("true");
    expect(useToast().items[0]?.message).toBe("Added to favourites");
  });

  it("shows a clear message instead of replacing the last slot when full", async () => {
    addFavorite.mockRejectedValue(
      new ApiError("You already have 3 favourites", 409, "You already have 3 favourites"),
    );
    const wrapper = await mountBook();
    const button = wrapper.findAll("button").find((node) => node.text().includes("Favourite"));
    expect(button).toBeTruthy();
    await button!.trigger("click");
    await flushPromises();
    expect(button!.text()).toBe("Favourite");
    expect(button!.attributes("aria-pressed")).toBe("false");
    expect(useToast().items[0]?.message).toBe(
      "You already have 3 favourites. Remove one first, or reorder in Settings.",
    );
  });
});
