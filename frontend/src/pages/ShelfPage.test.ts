import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { edition, favorite as favoriteRow, shelfItem, user } from "../test/fixtures";
import type { ClubSummary, Favorite, ShelfItem } from "../types";

const myShelf = vi.fn();
const clubs = vi.fn();
const patchShelf = vi.fn();
const removeFromShelf = vi.fn();
const addToShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    patchShelf: (...args: unknown[]) => patchShelf(...args),
    myShelf: (...args: unknown[]) => myShelf(...args),
    clubs: (...args: unknown[]) => clubs(...args),
    removeFromShelf: (...args: unknown[]) => removeFromShelf(...args),
    addToShelf: (...args: unknown[]) => addToShelf(...args),
  },
}));

import { useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import ShelfPage from "./ShelfPage.vue";
import ShelfBoard from "../components/ShelfBoard.vue";
import { i18n } from "../i18n";

function favorite(position: number, title: string): Favorite {
  return favoriteRow(position, { title, authors: "An Author" });
}

function item(id: number, title: string): ShelfItem {
  return shelfItem({
    id,
    updated_at: "2026-09-01T00:00:00Z",
    book: { title, authors: "An Author", year: 2018 },
  });
}

async function mountShelf(favorites: Favorite[], items: ShelfItem[] = [], memberOf: ClubSummary[] = []) {
  myShelf.mockResolvedValue({
    timezone: "UTC",
    user: { id: 1, username: "ada", avatar_url: null },
    items,
    favorites,
  });
  const pinia = createPinia();
  setActivePinia(pinia);
  const session = useSession();
  session.user = user({ clubs: memberOf });
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
    clubs.mockReset();
    patchShelf.mockReset();
    removeFromShelf.mockReset();
    addToShelf.mockReset();
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
    // The current status is marked as pressed and stays enabled; a tap on it
    // only closes the sheet.
    const current = wrapper.findAll(".sheet .status-choice").find(button => button.text() === "Want to read")!;
    expect(current.attributes("aria-pressed")).toBe("true");
    expect(current.attributes("disabled")).toBeUndefined();
    const calls = patchShelf.mock.calls.length;
    await current.trigger("click");
    expect(wrapper.find(".sheet").exists()).toBe(false);
    expect(patchShelf.mock.calls.length).toBe(calls);

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

  it("stacks each status filter's count under its label, and shows the counts only there", async () => {
    const reading = { ...item(2, "Circe"), status: "currently_reading" as const };
    const done = { ...item(3, "Dune"), status: "finished" as const };
    const wrapper = await mountShelf([], [item(1, "Middlemarch"), reading, done]);
    // The page head is the title alone: the filter below carries the numbers.
    expect(wrapper.find(".shelf-head .lede").exists()).toBe(false);
    const filter = wrapper.get(".shelf-filter");
    expect(filter.classes()).toContain("stacked");
    const segments = filter.findAll("button").map((button) => [button.get(".seg-label").text(), button.get(".seg-count").text()]);
    expect(segments).toEqual([["All", "3"], ["To read", "1"], ["Reading", "1"], ["Read", "1"], ["DNF", "0"]]);
    wrapper.unmount();
  });

  it("says only that an empty shelf is empty, next to the ways to fill it", async () => {
    const wrapper = await mountShelf([]);
    const empty = wrapper.get(".empty");
    expect(empty.get("h3").text()).toBe("Your shelf is empty");
    expect(empty.find("p").exists()).toBe(false);
    expect(empty.findAll(".btn").map((link) => link.text())).toEqual(["Find a book", "Import from Goodreads"]);
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

  it("makes no club request for a reader without clubs and offers no club actions", async () => {
    const wrapper = await mountShelf([], [item(1, "Circe")]);
    expect(clubs).not.toHaveBeenCalled();
    await wrapper.get("button.shelf-status").trigger("click");
    const labels = wrapper.findAll(".sheet button").map((button) => button.text());
    expect(labels).not.toContain("Set as club pick");
    expect(labels).not.toContain("Nominate for next up");
    expect(labels).toContain("Remove");
    wrapper.unmount();
  });

  it("marks the current picks of the reader's clubs and offers the actions their role allows", async () => {
    const club: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };
    clubs.mockResolvedValue({
      items: [{
        slug: club.slug, name: club.name, timezone: "UTC", role: "member", member_count: 3,
        created_at: "2026-01-01T00:00:00Z",
        current_pick: { id: 5, book: shelfItem({ id: 2 }).book, meeting_at: null },
      }],
    });
    const wrapper = await mountShelf([], [item(1, "Circe"), item(2, "Galatea")], [club]);
    expect(clubs).toHaveBeenCalledTimes(1);
    expect(wrapper.getComponent(ShelfBoard).props("clubPickIds")).toEqual([2]);
    await wrapper.findAll("button.shelf-status")[0].trigger("click");
    const labels = wrapper.findAll(".sheet button").map((button) => button.text());
    expect(labels).not.toContain("Set as club pick");
    expect(labels).toContain("Nominate for next up");
    wrapper.unmount();
  });

  it("removes a book and puts back its edition, rating, take and progress with Undo", async () => {
    const finished = shelfItem({
      id: 1,
      status: "finished",
      edition: edition({ id: 12, work_id: 1 }),
      rating: 5,
      rating_visibility: "only_me",
      take: "A storm of a book.",
      updated_at: "2026-09-01T00:00:00Z",
      book: { title: "Circe", authors: "An Author", year: 2018 },
    });
    const wrapper = await mountShelf([], [finished]);
    await wrapper.findAll("button.shelf-status")[0]!.trigger("click");
    await wrapper.findAll(".sheet button").find((button) => button.text() === "Remove")!.trigger("click");
    removeFromShelf.mockResolvedValue(undefined);
    await wrapper.findAll(".sheet .btn-danger").find((button) => button.text() === "Remove")!.trigger("click");
    await flushPromises();
    expect(removeFromShelf).toHaveBeenCalledWith(1);
    const toast = useToast().items.at(-1)!;
    expect(toast.message).toBe("Removed “Circe”");

    addToShelf.mockResolvedValue(finished);
    myShelf.mockResolvedValue({ timezone: "UTC", user: { id: 1, username: "ada", avatar_url: null }, items: [finished], favorites: [] });
    await useToast().run(toast.id);
    await flushPromises();
    expect(addToShelf).toHaveBeenCalledWith({
      work_id: 1,
      edition_id: 12,
      status: "finished",
      rating: 5,
      rating_visibility: "only_me",
      take: "A storm of a book.",
      dnf_reason: "",
      progress: null,
    });
    expect(myShelf).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  describe("finishing a book from the status sheet", () => {
    const club: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 };

    async function finishFirst(wrapper: Awaited<ReturnType<typeof mountShelf>>) {
      await wrapper.findAll("button.shelf-status")[0].trigger("click");
      await wrapper.findAll(".sheet .status-choice").find((button) => button.text() === "Finished")!.trigger("click");
      await flushPromises();
    }

    it("opens the moment instead of the toast, with the club whose pick it is, and shows the stars it saved", async () => {
      clubs.mockResolvedValue({
        items: [{
          slug: club.slug, name: club.name, timezone: "UTC", role: "member", member_count: 3,
          created_at: "2026-01-01T00:00:00Z",
          current_pick: { id: 5, book: shelfItem({ id: 1 }).book, meeting_at: null },
        }],
      });
      const reading = { ...item(1, "Circe"), status: "currently_reading" as const, progress: 80 };
      const wrapper = await mountShelf([], [reading], [club]);
      const done = { ...reading, status: "finished" as const, progress: null, finished_at: "2026-11-06T10:00:00Z" };
      patchShelf.mockResolvedValueOnce(done);
      await finishFirst(wrapper);
      expect(patchShelf).toHaveBeenLastCalledWith(1, { status: "finished", position: 0 });
      expect(wrapper.find(".sheet").exists()).toBe(false);
      const finish = useFinish();
      expect(finish.item).toStrictEqual(done);
      expect(finish.clubSlug).toBe(club.slug);
      expect(useToast().items.map((toast) => toast.message)).not.toContain("Moved to Finished");

      finish.close({ ...done, rating: 5 });
      await flushPromises();
      expect(wrapper.getComponent(ShelfBoard).props("items")[0]).toMatchObject({ id: 1, status: "finished", rating: 5 });
      wrapper.unmount();
    });

    it("keeps the toast and opens no moment for any other status", async () => {
      const finished = { ...item(1, "Circe"), status: "finished" as const, rating: 4 };
      const wrapper = await mountShelf([], [finished]);
      await wrapper.findAll("button.shelf-status")[0].trigger("click");
      patchShelf.mockResolvedValueOnce({ ...finished, status: "currently_reading", rating: null });
      await wrapper.findAll(".sheet .status-choice").find((button) => button.text() === "Reading")!.trigger("click");
      await flushPromises();
      expect(useFinish().item).toBeNull();
      expect(useToast().items.at(-1)?.message).toBe("Moved to Reading");
      wrapper.unmount();
    });

    it("opens no moment when the server answers with another status", async () => {
      const wrapper = await mountShelf([], [item(1, "Circe")]);
      patchShelf.mockResolvedValueOnce({ ...item(1, "Circe"), status: "want_to_read" });
      await finishFirst(wrapper);
      expect(useFinish().item).toBeNull();
      wrapper.unmount();
    });
  });
});
