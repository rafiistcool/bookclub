import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookDetail, edition, shelfItem, user } from "../test/fixtures";
import type { BookDetail, ClubSummary } from "../types";

const book = vi.fn();
const addFavorite = vi.fn();
const removeFavorite = vi.fn();
const addToShelf = vi.fn();
const patchShelf = vi.fn();
const removeFromShelf = vi.fn();
const editions = vi.fn();
const requestWorkEdit = vi.fn();
const diary = vi.fn();
const nominate = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    item?: unknown;
    constructor(message: string, status: number, _detail?: unknown, body?: { item?: unknown }) {
      super(message);
      this.name = "ApiError";
      this.status = status;
      this.item = body?.item;
    }
  },
  api: {
    book: (...args: unknown[]) => book(...args),
    addFavorite: (...args: unknown[]) => addFavorite(...args),
    removeFavorite: (...args: unknown[]) => removeFavorite(...args),
    addToShelf: (...args: unknown[]) => addToShelf(...args),
    patchShelf: (...args: unknown[]) => patchShelf(...args),
    removeFromShelf: (...args: unknown[]) => removeFromShelf(...args),
    editions: (...args: unknown[]) => editions(...args),
    requestWorkEdit: (...args: unknown[]) => requestWorkEdit(...args),
    diary: (...args: unknown[]) => diary(...args),
    nominate: (...args: unknown[]) => nominate(...args),
  },
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import BookDetailPage from "./BookDetailPage.vue";

function detail(overrides: Partial<BookDetail> = {}): BookDetail {
  return bookDetail({
    id: 4,
    ol_work_key: null,
    title: "The Witch of Portobello",
    authors: "Paulo Coelho",
    year: 2006,
    source: "user",
    custom: true,
    can_refresh: false,
    ...overrides,
  });
}

async function mountBook(path = "/book/4", clubs: ClubSummary[] = []) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/book/:id", component: BookDetailPage }],
  });
  await router.push(path);
  await router.isReady();
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  const wrapper = mount(BookDetailPage, {
    global: { plugins: [pinia, router] },
  });
  await flushPromises();
  return { wrapper, router };
}

function button(wrapper: Awaited<ReturnType<typeof mountBook>>["wrapper"], label: string) {
  return wrapper.findAll("button").find((node) => node.text().includes(label));
}

describe("BookDetailPage", () => {
  beforeEach(() => {
    for (const mock of [book, addFavorite, removeFavorite, addToShelf, patchShelf, removeFromShelf, editions, requestWorkEdit, diary, nominate]) {
      mock.mockReset();
    }
    book.mockResolvedValue(detail());
    diary.mockResolvedValue({ work_id: 4, my_progress: null, my_status: null, items: [], timezone: "UTC" });
    editions.mockResolvedValue({ items: [] });
    i18n.global.locale.value = "en";
  });

  it("adds a favourite when there is a free slot", async () => {
    addFavorite.mockResolvedValue({
      items: [{ position: 1, book: { id: 4, title: "The Witch of Portobello" } }],
    });
    const { wrapper } = await mountBook();
    const favourite = button(wrapper, "Favourite");
    expect(favourite).toBeTruthy();
    await favourite!.trigger("click");
    await flushPromises();
    expect(addFavorite).toHaveBeenCalledWith(4);
    expect(favourite!.text()).toContain("Favourite · 1");
    expect(favourite!.attributes("aria-pressed")).toBe("true");
    expect(useToast().items[0]?.message).toBe("Added to favourites");
  });

  it("shows a clear message instead of replacing the last slot when full", async () => {
    addFavorite.mockRejectedValue(
      new ApiError("You already have 3 favourites", 409, "You already have 3 favourites"),
    );
    const { wrapper } = await mountBook();
    const favourite = button(wrapper, "Favourite");
    await favourite!.trigger("click");
    await flushPromises();
    expect(favourite!.text()).toBe("Favourite");
    expect(favourite!.attributes("aria-pressed")).toBe("false");
    expect(useToast().items[0]?.message).toBe(
      "You already have 3 favourites. Remove one first, or reorder in Settings.",
    );
  });

  it("resolves a legacy link to the work's id and keeps the diary anchor", async () => {
    book.mockResolvedValue(detail({ id: 17, ol_work_key: "/works/OL1168007W", custom: false }));
    const { wrapper, router } = await mountBook("/book/OL1168007W#entry-11");
    expect(book).toHaveBeenCalledTimes(1);
    expect(book.mock.calls[0][0]).toBe("OL1168007W");
    expect(router.currentRoute.value.fullPath).toBe("/book/17#entry-11");
    expect(wrapper.text()).toContain("The Witch of Portobello");
    expect(diary).toHaveBeenCalledWith(17);
  });

  it("follows a merged work to the id it now lives under", async () => {
    book.mockResolvedValue(detail({ id: 9 }));
    const { router } = await mountBook("/book/4");
    expect(book.mock.calls[0][0]).toBe(4);
    expect(router.currentRoute.value.fullPath).toBe("/book/9");
    expect(book).toHaveBeenCalledTimes(1);
  });

  it("says the book is not in the catalog for an id that cannot be one", async () => {
    const { wrapper } = await mountBook("/book/not-a-book");
    expect(book).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("That book is not in the catalog.");
    // Another try asks nothing new: only the way back.
    expect(button(wrapper, "Try again")).toBeUndefined();
    expect(wrapper.find('a[href="/discover"].btn').text()).toBe("Back to Discover");
  });

  it("offers no second try when the server has no such book", async () => {
    book.mockRejectedValue(new ApiError("That book is not in the catalog.", 404, "No such book."));
    const { wrapper } = await mountBook("/book/999999");
    expect(wrapper.text()).toContain("That book is not in the catalog.");
    expect(button(wrapper, "Try again")).toBeUndefined();
    expect(wrapper.find('a[href="/discover"].btn').exists()).toBe(true);
  });

  it("offers a second try after a network or server failure, and it can succeed", async () => {
    book.mockRejectedValueOnce(new TypeError("Failed to fetch"));
    const { wrapper } = await mountBook("/book/4");
    expect(wrapper.text()).toContain("Could not load that book");
    const retry = button(wrapper, "Try again");
    expect(retry).toBeTruthy();
    await retry!.trigger("click");
    await flushPromises();
    expect(book).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("The Witch of Portobello");

    book.mockRejectedValueOnce(new ApiError("Could not load the library right now.", 502, "down"));
    const second = await mountBook("/book/4");
    expect(button(second.wrapper, "Try again")).toBeTruthy();
  });

  it("shelves a book by its work id and shows what the server stored", async () => {
    addToShelf.mockResolvedValue(shelfItem({ id: 30, status: "finished", rating: null, book: { id: 4 } }));
    const { wrapper } = await mountBook();
    await wrapper.findAll(".status-control button").find((node) => node.text() === "Done")!.trigger("click");
    await flushPromises();
    expect(addToShelf).toHaveBeenCalledWith({ work_id: 4, status: "finished" });
    expect(wrapper.find(".star-row").exists()).toBe(true);
  });

  it("offers no club actions without clubs, and only nominating to a plain member", async () => {
    const solo = await mountBook();
    expect(button(solo.wrapper, "Set as club pick")).toBeUndefined();
    expect(button(solo.wrapper, "Nominate")).toBeUndefined();
    solo.wrapper.unmount();

    const club: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };
    nominate.mockResolvedValue({});
    const member = await mountBook("/book/4", [club]);
    expect(button(member.wrapper, "Set as club pick")).toBeUndefined();
    await button(member.wrapper, "Nominate")!.trigger("click");
    await flushPromises();
    expect(nominate).toHaveBeenCalledWith("readers-ab12", 4);
    expect(useToast().items.at(-1)?.message).toBe("Nominated in Readers");
  });

  it("links to the cover report only for a stored cover", async () => {
    const plain = await mountBook();
    expect(button(plain.wrapper, "Report cover")).toBeUndefined();
    plain.wrapper.unmount();
    book.mockResolvedValue(detail({ cover_url: "/media/abc/l", cover_thumb_url: "/media/abc/s", cover_public_id: "abc" }));
    const covered = await mountBook();
    await button(covered.wrapper, "Report cover")!.trigger("click");
    expect(covered.wrapper.text()).toContain("Report this cover");
  });

  it("suggests a correction of the book's details with only what changed", async () => {
    book.mockResolvedValue(detail({ subjects: ["Fiction"], pages: null }));
    const { wrapper } = await mountBook();
    await button(wrapper, "Wrong details?")!.trigger("click");
    const form = wrapper.get("form.edit-request");
    expect((form.get("input[name=title]").element as HTMLInputElement).value).toBe("The Witch of Portobello");
    await form.trigger("submit");
    expect(requestWorkEdit).not.toHaveBeenCalled();
    expect(form.get(".error").text()).toBe("Change at least one field first.");

    await form.get("input[name=pages]").setValue("320");
    await form.get("textarea[name=subjects]").setValue("Fiction\nBrazil");
    requestWorkEdit.mockResolvedValue({ id: 3, status: "open" });
    await form.trigger("submit");
    await flushPromises();
    expect(requestWorkEdit).toHaveBeenCalledWith(4, { pages: 320, subjects: ["Fiction", "Brazil"] }, "");
    expect(wrapper.find("form.edit-request").exists()).toBe(false);
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at it.");
  });

  describe("a finished book on the shelf", () => {
    const FINISHED = {
      shelf_id: 30,
      on_shelf: "finished" as const,
      edition_id: 12,
      rating: 4,
      rating_visibility: "audience" as const,
      take: "Sharp and kind.",
      dnf_reason: "",
      progress: null,
    };
    const stored = (overrides: Parameters<typeof shelfItem>[0] = {}) =>
      shelfItem({
        id: 30,
        status: "finished",
        rating: 4,
        rating_visibility: "audience",
        take: "Sharp and kind.",
        book: { id: 4 },
        ...overrides,
      });
    const onlyMeBox = (wrapper: Awaited<ReturnType<typeof mountBook>>["wrapper"]) =>
      wrapper.get(".check-row input[type=checkbox]").element as HTMLInputElement;

    beforeEach(() => {
      book.mockResolvedValue(detail(FINISHED));
    });

    it("keeps the rating to the reader with “Only me”, and shares it again", async () => {
      const { wrapper } = await mountBook();
      expect(onlyMeBox(wrapper).checked).toBe(false);

      patchShelf.mockResolvedValue(stored({ rating_visibility: "only_me" }));
      await wrapper.get(".check-row input[type=checkbox]").setValue(true);
      await flushPromises();
      expect(patchShelf).toHaveBeenLastCalledWith(30, { rating_visibility: "only_me" });
      expect(onlyMeBox(wrapper).checked).toBe(true);
      expect(useToast().items.at(-1)?.message).toBe("Rating visibility saved");

      patchShelf.mockResolvedValue(stored({ rating_visibility: "audience" }));
      await wrapper.get(".check-row input[type=checkbox]").setValue(false);
      await flushPromises();
      expect(patchShelf).toHaveBeenLastCalledWith(30, { rating_visibility: "audience" });
      expect(onlyMeBox(wrapper).checked).toBe(false);
    });

    it("shows the stored visibility again when the save fails", async () => {
      book.mockResolvedValue(detail({ ...FINISHED, rating_visibility: "only_me" }));
      const { wrapper } = await mountBook();
      expect(onlyMeBox(wrapper).checked).toBe(true);
      patchShelf.mockRejectedValue(new ApiError("Book not on your shelf", 404, "Book not on your shelf"));
      await wrapper.get(".check-row input[type=checkbox]").setValue(false);
      await flushPromises();
      expect(patchShelf).toHaveBeenCalledWith(30, { rating_visibility: "audience" });
      expect(onlyMeBox(wrapper).checked).toBe(true);
    });

    it("removes the book and puts back everything with Undo", async () => {
      book.mockResolvedValue(detail({ ...FINISHED, rating_visibility: "only_me" }));
      const { wrapper } = await mountBook();
      removeFromShelf.mockResolvedValue(undefined);
      await button(wrapper, "Remove")!.trigger("click");
      await flushPromises();
      expect(removeFromShelf).toHaveBeenCalledWith(30);
      expect(wrapper.find(".star-row").exists()).toBe(false);
      const toast = useToast().items.at(-1)!;
      expect(toast.message).toBe("Removed from your shelf");
      expect(toast.action?.label).toBe("Undo");

      addToShelf.mockResolvedValue(
        stored({ id: 31, rating_visibility: "only_me", edition: edition({ id: 12, work_id: 4 }) }),
      );
      await useToast().run(toast.id);
      await flushPromises();
      expect(addToShelf).toHaveBeenCalledWith({
        work_id: 4,
        edition_id: 12,
        status: "finished",
        rating: 4,
        rating_visibility: "only_me",
        take: "Sharp and kind.",
        dnf_reason: "",
        progress: null,
      });
      expect(wrapper.find(".star-row").exists()).toBe(true);
      expect(onlyMeBox(wrapper).checked).toBe(true);
    });
  });
});
