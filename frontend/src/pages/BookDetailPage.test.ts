import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookDetail, bookOut, edition, shelfItem, user } from "../test/fixtures";
import type { BookDetail, ClubSummary } from "../types";

// No scanner is mounted here: the cover scan hands each photo back as it is.
vi.mock("../stores/coverScan", () => import("../test/coverScan"));

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
const uploadCover = vi.fn();
const adminSetCover = vi.fn();
const refreshBook = vi.fn();
const coverCandidates = vi.fn();
const chooseCover = vi.fn();
const uploadPersonalCover = vi.fn();
const clearPersonalCover = vi.fn();

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
    uploadCover: (...args: unknown[]) => uploadCover(...args),
    adminSetCover: (...args: unknown[]) => adminSetCover(...args),
    refreshBook: (...args: unknown[]) => refreshBook(...args),
    coverCandidates: (...args: unknown[]) => coverCandidates(...args),
    chooseCover: (...args: unknown[]) => chooseCover(...args),
    uploadPersonalCover: (...args: unknown[]) => uploadPersonalCover(...args),
    clearPersonalCover: (...args: unknown[]) => clearPersonalCover(...args),
  },
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import ProgressEditor from "../components/ProgressEditor.vue";
import { useFinish } from "../stores/finish";
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

async function mountBook(path = "/book/4", clubs: ClubSummary[] = [], isAdmin = false, attach = false) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/book/:id", component: BookDetailPage }],
  });
  await router.push(path);
  await router.isReady();
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs, is_admin: isAdmin });
  const wrapper = mount(BookDetailPage, {
    global: { plugins: [pinia, router] },
    ...(attach ? { attachTo: document.body } : {}),
  });
  await flushPromises();
  return { wrapper, router };
}

function button(wrapper: Awaited<ReturnType<typeof mountBook>>["wrapper"], label: string) {
  return wrapper.findAll("button").find((node) => node.text().includes(label));
}

type Page = Awaited<ReturnType<typeof mountBook>>["wrapper"];

/** Open the "…" sheet, where the book's other actions are. */
async function openMore(wrapper: Page) {
  await wrapper.get("button.more-btn").trigger("click");
  await flushPromises();
}

/** The group headings of the "…" sheet. */
function headings(wrapper: Page) {
  return wrapper.findAll(".book-actions h3").map((node) => node.text());
}

/** Choose `file` in the file input inside `selector`. */
async function choose(wrapper: Page, selector: string, file: File) {
  const input = wrapper.get(`${selector} input[type=file]`);
  Object.defineProperty(input.element, "files", { value: [file], configurable: true });
  await input.trigger("change");
  await flushPromises();
}

describe("BookDetailPage", () => {
  beforeEach(() => {
    for (const mock of [book, addFavorite, removeFavorite, addToShelf, patchShelf, removeFromShelf, editions, requestWorkEdit, diary, nominate, uploadCover, adminSetCover, refreshBook, coverCandidates, chooseCover, uploadPersonalCover, clearPersonalCover]) {
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
      "You already have 3 favourites. Remove one first, or reorder on your profile.",
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
    await wrapper.findAll(".status-control button").find((node) => node.text() === "Read")!.trigger("click");
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

  it("keeps the hero short: a club badge, author · year, and one rating line with the locale's comma", async () => {
    i18n.global.locale.value = "de";
    const readers = { slug: "readers-ab12", name: "Leserunde" };
    book.mockResolvedValue(detail({ club_picks: [readers], club_rating: 4.25, rating_count: 3 }));
    const one = await mountBook();
    expect(one.wrapper.find(".kicker").exists()).toBe(false);
    expect(one.wrapper.get(".hero-pick").text()).toBe("Club-Buch · Leserunde");
    expect(one.wrapper.get(".detail-authors").text()).toBe("Paulo Coelho · 2006");
    expect(one.wrapper.get(".detail-authors .nums").attributes("title")).toBe("Erstveröffentlichung 2006");
    expect(one.wrapper.text()).not.toContain("Erstveröffentlichung");
    const rating = one.wrapper.get(".club-rating");
    expect(rating.get("strong").text()).toBe("4,3");
    expect(rating.text()).toContain("· 3 Stimmen");
    // What the line was moved out of the hero for: the reader's note and the links.
    expect(one.wrapper.find(".detail-intro").text()).not.toContain("Von Hand angelegt");
    expect(one.wrapper.find(".detail-intro a").exists()).toBe(false);
    one.wrapper.unmount();

    i18n.global.locale.value = "en";
    book.mockResolvedValue(detail({ club_picks: [readers, { slug: "night-cd34", name: "Night owls" }], year: null }));
    const two = await mountBook();
    const badge = two.wrapper.get(".hero-pick .badge");
    // Every club by name, readable on a phone (no tooltip).
    expect(badge.text()).toBe("Club pick · Leserunde, Night owls");
    expect(badge.attributes("title")).toBeUndefined();
    expect(two.wrapper.get(".detail-authors").text()).toBe("Paulo Coelho");
    expect(two.wrapper.find(".club-rating").exists()).toBe(false);
  });

  it("draws the shelf panel without a card, its other actions as light rows and Remove last", async () => {
    const club: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "admin", timezone: "UTC", member_count: 3 };
    book.mockResolvedValue(detail({ shelf_id: 30, on_shelf: "want_to_read" }));
    const { wrapper } = await mountBook("/book/4", [club]);
    const panel = wrapper.get(".shelf-panel");
    expect(panel.classes()).not.toContain("panel");
    expect(panel.find(".btn-sm").exists()).toBe(false);
    expect(panel.findAll(".panel-actions button.sheet-row").map((node) => node.text())).toEqual([
      "Favourite",
      "Set as club pick",
      "Nominate for next up",
    ]);
    const remove = panel.get("button.remove-btn");
    expect(remove.text()).toBe("Remove");
    expect(remove.classes()).toEqual(expect.arrayContaining(["text-btn", "danger"]));
  });

  it("lists other readers as whole-row links with the locale's percent", async () => {
    i18n.global.locale.value = "de";
    book.mockResolvedValue(
      detail({
        readers: [
          { username: "mara", status: "currently_reading", progress: 35, rating: null, avatar_url: null },
          { username: "jonas", status: "finished", progress: null, rating: 4, avatar_url: null },
        ],
      }),
    );
    const { wrapper } = await mountBook();
    const rows = wrapper.findAll(".reader-list li");
    expect(rows).toHaveLength(2);
    const link = rows[0]!.get("a.reader-row");
    expect(link.attributes("href")).toBe("/u/mara");
    // Avatar, name, badge and progress all sit inside the one link.
    expect(link.find(".avatar").exists()).toBe(true);
    expect(link.get(".reader-name").text()).toBe("mara");
    expect(link.text()).toContain("35 %");
  });

  it("steps a very long title down a size and keeps each author's name whole", async () => {
    book.mockResolvedValue(
      detail({
        title: "Die unglaubliche, aber größtenteils wahre Geschichte der verschwundenen Leihbücher",
        authors: "Annegret Feldmann, Jo Brandt",
        year: 2019,
      }),
    );
    const { wrapper } = await mountBook();
    expect(wrapper.get("h1.display").classes()).toContain("long-title");
    expect(wrapper.findAll(".detail-authors .author-name").map((node) => node.text())).toEqual([
      "Annegret Feldmann",
      "Jo Brandt",
    ]);
    expect(wrapper.get(".detail-authors").text()).toBe("Annegret Feldmann, Jo Brandt · 2019");
    wrapper.unmount();

    book.mockResolvedValue(detail());
    const short = await mountBook();
    expect(short.wrapper.get("h1.display").classes()).not.toContain("long-title");
  });

  it("holds the page's shape while it loads: title, status control and panel lines", async () => {
    book.mockReturnValue(new Promise(() => {}));
    const { wrapper } = await mountBook();
    const skeleton = wrapper.get(".detail-skeleton");
    expect(skeleton.attributes("aria-hidden")).toBe("true");
    expect(skeleton.find(".skeleton-title").exists()).toBe(true);
    expect(skeleton.find(".skeleton-panel .skeleton-status").exists()).toBe(true);
    expect(skeleton.findAll(".skeleton-panel .skeleton-line")).toHaveLength(2);
  });

  it("gathers the book's other actions behind one “…” button", async () => {
    book.mockResolvedValue(detail({ custom: false, source: "openlibrary", ol_work_key: "/works/OL4W", can_refresh: true }));
    const { wrapper } = await mountBook();
    const more = wrapper.get("button.more-btn");
    expect(more.attributes("aria-label")).toBe("More for this book");
    expect(more.attributes("aria-haspopup")).toBe("dialog");
    expect(more.attributes("aria-expanded")).toBe("false");
    expect(button(wrapper, "Wrong details?")).toBeUndefined();

    await openMore(wrapper);
    expect(more.attributes("aria-expanded")).toBe("true");
    const sheet = wrapper.get(".book-actions .sheet");
    expect(sheet.attributes("aria-label")).toBe("The Witch of Portobello");
    expect(headings(wrapper)).toEqual(["About this book"]);
    const link = sheet.get("a[href]");
    expect(link.text()).toBe("View on Open Library");
    expect(link.attributes("href")).toBe("https://openlibrary.org/works/OL4W");
    expect(link.attributes("target")).toBe("_blank");
    expect(link.attributes("rel")).toBe("noreferrer");
    // Light rows like the club's "More" sheet, under a sentence-case head (no caps kicker).
    expect(sheet.find(".caps").exists()).toBe(false);
    expect(sheet.find(".btn").exists()).toBe(false);
    expect(sheet.findAll("button.sheet-row").map((node) => node.text())).toEqual([
      "Refresh details",
      "Wrong details?",
      "Same book as another one?",
    ]);
    // A catalog book is corrected, not reported.
    expect(sheet.find(".report-book").exists()).toBe(false);

    await sheet.get(".sheet-close").trigger("click");
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(more.attributes("aria-expanded")).toBe("false");
  });

  it("offers a book a reader added its note and report, without Open Library", async () => {
    const { wrapper } = await mountBook();
    await openMore(wrapper);
    const sheet = wrapper.get(".book-actions");
    expect(sheet.text()).toContain("Added by a reader — not on Open Library.");
    expect(sheet.find("a[href]").exists()).toBe(false);
    expect(sheet.find(".refresh-details").exists()).toBe(false);
    expect(sheet.get("button.report-book").text()).toBe("Report this book");
  });

  it("shows “Refreshing…” in the sheet until the answer is in, then closes it", async () => {
    book.mockResolvedValue(detail({ custom: false, source: "openlibrary", ol_work_key: "/works/OL4W", can_refresh: true }));
    const { wrapper } = await mountBook();
    let answer!: (value: BookDetail) => void;
    refreshBook.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    await openMore(wrapper);
    await wrapper.get(".refresh-details").trigger("click");
    expect(wrapper.get(".refresh-details").text()).toBe("Refreshing…");
    expect(wrapper.get(".refresh-details").attributes("disabled")).toBeDefined();
    answer(detail({ custom: false, source: "openlibrary", ol_work_key: "/works/OL4W", title: "The Witch of Portobello (new)" }));
    await flushPromises();
    expect(refreshBook).toHaveBeenCalledWith(4);
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(wrapper.get("h1").text()).toBe("The Witch of Portobello (new)");
    expect(useToast().items.at(-1)?.message).toBe("Details updated");
  });

  it("opens the next sheet only once the “…” sheet is gone, and gives focus back to “…”", async () => {
    const { wrapper } = await mountBook("/book/4", [], false, true);
    const more = wrapper.get("button.more-btn");
    (more.element as HTMLButtonElement).focus();
    await openMore(wrapper);
    await button(wrapper, "Wrong details?")!.trigger("click");
    await flushPromises();
    // Never a sheet on a sheet: one Escape and one Tab trap at a time.
    expect(wrapper.findAll(".sheet")).toHaveLength(1);
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(wrapper.find("form.edit-request").exists()).toBe(true);

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flushPromises();
    expect(wrapper.findAll(".sheet")).toHaveLength(0);
    expect(document.activeElement).toBe(more.element);

    // Escape on the "…" sheet itself closes it, and focus stays on "…".
    await openMore(wrapper);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flushPromises();
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(document.activeElement).toBe(more.element);
    wrapper.unmount();
  });

  it("puts the cover report into the sheet, only for a stored cover", async () => {
    const plain = await mountBook();
    await openMore(plain.wrapper);
    expect(headings(plain.wrapper)).not.toContain("Cover");
    expect(button(plain.wrapper, "Report cover")).toBeUndefined();
    plain.wrapper.unmount();
    book.mockResolvedValue(detail({ cover_url: "/media/abc/l", cover_thumb_url: "/media/abc/s", cover_public_id: "abc" }));
    const covered = await mountBook();
    // With a cover there is no link under it: the sheet suggests a photo instead.
    expect(covered.wrapper.find(".hero-cover .cover-photo").exists()).toBe(false);
    await openMore(covered.wrapper);
    expect(headings(covered.wrapper)).toEqual(["About this book", "Cover"]);
    expect(covered.wrapper.get(".book-actions .suggest-photo").text()).toBe("Suggest a photo of your copy");
    await button(covered.wrapper, "Report cover")!.trigger("click");
    await flushPromises();
    expect(covered.wrapper.findAll(".sheet")).toHaveLength(1);
    expect(covered.wrapper.get(".sheet").text()).toContain("Report this cover");
  });

  it("uploads a photo chosen in the sheet after the sheet has closed", async () => {
    book.mockResolvedValue(detail({ cover_url: "/media/abc/l", cover_thumb_url: "/media/abc/s", cover_public_id: "abc", edition_id: 12 }));
    const { wrapper } = await mountBook();
    let answer!: (value: unknown) => void;
    uploadCover.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    await openMore(wrapper);
    const photo = new File(["x"], "front.jpg", { type: "image/jpeg" });
    await choose(wrapper, ".book-actions .suggest-photo", photo);
    // The sheet goes at once; the cover says the photo is on its way.
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(uploadCover).toHaveBeenCalledWith(4, photo, 12);
    expect(wrapper.get(".hero-cover").attributes("aria-busy")).toBe("true");
    expect(wrapper.get(".cover-status").text()).toBe("Uploading…");
    answer({ request_id: 7 });
    await flushPromises();
    expect(wrapper.find(".cover-status").exists()).toBe(false);
    expect(wrapper.get(".hero-cover").attributes("aria-busy")).toBe("false");
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at your photo.");
  });

  it("asks a reader with a catalog book on the shelf for a photo in place of the small link", async () => {
    const shelved = { custom: false, source: "legacy" as const, shelf_id: 30, on_shelf: "want_to_read" as const, edition_id: 12 };
    // Off the shelf: only the small link under the cloth.
    book.mockResolvedValue(detail({ custom: false, source: "legacy" }));
    const browsing = await mountBook();
    expect(browsing.wrapper.find(".cover-prompt").exists()).toBe(false);
    expect(browsing.wrapper.get(".hero-cover .cover-photo").text()).toBe("Add a photo of your copy");
    browsing.wrapper.unmount();

    book.mockResolvedValue(detail(shelved));
    const { wrapper } = await mountBook();
    expect(wrapper.find(".hero-cover .cover-photo").exists()).toBe(false);
    const prompt = wrapper.get(".cover-prompt");
    expect(prompt.text()).toContain("This book has no cover yet");
    const covered = bookOut({ id: 4, cover_url: "/media/new/l", cover_thumb_url: "/media/new/s", cover_public_id: "new" });
    uploadCover.mockResolvedValue({ book: covered });
    const photo = new File(["x"], "front.jpg", { type: "image/jpeg" });
    await choose(wrapper, ".cover-prompt", photo);
    expect(uploadCover).toHaveBeenCalledWith(4, photo, 12);
    // The page shows the new cover: the prompt goes, and the sheet can report it or take a better photo.
    expect(wrapper.find(".cover-prompt").exists()).toBe(false);
    expect(wrapper.find(".hero-cover .cover-photo").exists()).toBe(false);
    await openMore(wrapper);
    expect(button(wrapper, "Report cover")).toBeTruthy();
    expect(wrapper.get(".book-actions .suggest-photo").text()).toBe("Suggest a photo of your copy");
  });

  it("leaves a book a reader added with the small link, on the shelf or not", async () => {
    book.mockResolvedValue(detail({ shelf_id: 30, on_shelf: "finished" }));
    const { wrapper } = await mountBook();
    expect(wrapper.find(".cover-prompt").exists()).toBe(false);
    expect(wrapper.get(".hero-cover .cover-photo").text()).toBe("Add a photo of your copy");
  });

  it("lets an instance admin set the book's cover, or any edition's, directly", async () => {
    const reader = await mountBook();
    await openMore(reader.wrapper);
    expect(headings(reader.wrapper)).not.toContain("Admin");
    expect(reader.wrapper.find(".admin-set-cover").exists()).toBe(false);
    reader.wrapper.unmount();

    // A page without an edition shows the work's cover: the answer is adopted.
    book.mockResolvedValue(detail({ custom: false, source: "dnb", shelf_id: 30, on_shelf: "finished" }));
    const { wrapper } = await mountBook("/book/4", [], true);
    await openMore(wrapper);
    // On the shelf, the cover group holds the reader's own choice.
    expect(headings(wrapper)).toEqual(["About this book", "Cover", "Admin"]);
    expect(wrapper.get(".admin-set-cover").text()).toBe("Set cover as admin");
    expect(wrapper.get(".admin-set-edition-cover").text()).toBe("Set an edition's cover as admin");
    // No edition on the shelf, nothing of the viewer's to edit.
    expect(button(wrapper, "Edit your edition as admin")).toBeUndefined();

    const covered = bookOut({ id: 4, cover_url: "/media/adm/l", cover_thumb_url: "/media/adm/s", cover_public_id: "adm" });
    adminSetCover.mockResolvedValue({ book: covered });
    const picture = new File(["x"], "cover.png", { type: "image/png" });
    book.mockClear();
    await choose(wrapper, ".admin-set-cover", picture);
    expect(adminSetCover).toHaveBeenCalledWith(4, picture, null);
    expect(useToast().items.at(-1)?.message).toBe("The cover is set for everyone");
    expect(book).not.toHaveBeenCalled();
    // The sheet closed on the pick, and the page still took the answer.
    expect(wrapper.find(".book-actions").exists()).toBe(false);
    expect(wrapper.get(".hero-cover img").attributes("src")).toContain("/media/adm/");
    expect(wrapper.find(".cover-prompt").exists()).toBe(false);
    wrapper.unmount();
  });

  it("reads the book again after an admin's work cover on a page in the viewer's edition", async () => {
    // The edition has its own cover, which the page shows: the work's new cover is not it.
    const own = { cover_url: "/media/ed/l", cover_thumb_url: "/media/ed/s", cover_public_id: "ed" };
    book.mockResolvedValue(detail({ custom: false, source: "dnb", shelf_id: 30, on_shelf: "finished", edition_id: 12, ...own }));
    const { wrapper } = await mountBook("/book/4", [], true);
    adminSetCover.mockResolvedValue({
      book: bookOut({ id: 4, cover_url: "/media/adm/l", cover_thumb_url: "/media/adm/s", cover_public_id: "adm" }),
    });
    book.mockClear();
    await openMore(wrapper);
    const picture = new File(["x"], "cover.png", { type: "image/png" });
    await choose(wrapper, ".admin-set-cover", picture);
    expect(adminSetCover).toHaveBeenCalledWith(4, picture, null);
    expect(book).toHaveBeenCalledWith(4);
    expect(wrapper.get(".hero-cover img").attributes("src")).toContain("/media/ed/");
  });

  it("lets an instance admin pick any edition for its cover", async () => {
    book.mockResolvedValue(detail({ custom: false, source: "dnb", shelf_id: 30, on_shelf: "finished", edition_id: 12 }));
    editions.mockResolvedValue({
      items: [
        edition({ id: 12, work_id: 4, format: "paperback", publisher: "Diogenes" }),
        edition({ id: 13, work_id: 4, format: "hardcover", publisher: "Hanser", cover_url: "/media/h/l", cover_thumb_url: "/media/h/s" }),
      ],
    });
    const { wrapper } = await mountBook("/book/4", [], true);
    await openMore(wrapper);
    await wrapper.get(".admin-set-edition-cover").trigger("click");
    await flushPromises();
    // The edition list replaces the "…" sheet instead of opening on top of it.
    expect(wrapper.findAll(".sheet")).toHaveLength(1);
    expect(wrapper.get(".sheet").attributes("aria-label")).toBe("An edition's cover");
    expect(editions).toHaveBeenCalledWith(4);
    const rows = wrapper.findAll(".edition-cover-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain("Your edition");
    expect(rows[1]!.text()).not.toContain("Your edition");

    book.mockClear();
    adminSetCover.mockResolvedValue({ book: bookOut({ id: 4 }) });
    const input = rows[1]!.get("input[type=file]");
    const picture = new File(["x"], "cover.png", { type: "image/png" });
    Object.defineProperty(input.element, "files", { value: [picture], configurable: true });
    await input.trigger("change");
    await flushPromises();
    expect(adminSetCover).toHaveBeenLastCalledWith(4, picture, 13);
    expect(book).toHaveBeenCalledWith(4);
    expect(wrapper.find(".edition-cover-row").exists()).toBe(false);
    expect(wrapper.findAll(".sheet")).toHaveLength(0);
  });

  it("suggests a correction of the book's details with only what changed", async () => {
    book.mockResolvedValue(detail({ subjects: ["Fiction"], pages: null }));
    const { wrapper } = await mountBook();
    await openMore(wrapper);
    await button(wrapper, "Wrong details?")!.trigger("click");
    await flushPromises();
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
      // "Remove", not "Remove rating".
      await wrapper.findAll("button").find((node) => node.text() === "Remove")!.trigger("click");
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

  describe("finishing the book", () => {
    const readers = { slug: "readers-ab12", name: "Readers" };
    const finishedItem = (overrides: Parameters<typeof shelfItem>[0] = {}) =>
      shelfItem({ id: 30, status: "finished", rating: null, book: { id: 4 }, ...overrides });
    const readButton = (wrapper: Page) =>
      wrapper.findAll(".status-control button").find((node) => node.text() === "Read")!;

    it("opens the moment with the pick's club instead of the toast, and shows the stars it saved", async () => {
      book.mockResolvedValue(detail({ club_picks: [readers], shelf_id: 30, on_shelf: "currently_reading", progress: 70 }));
      const { wrapper } = await mountBook();
      // No caps caption over the shelf controls.
      expect(wrapper.find(".shelf-panel .caps").exists()).toBe(false);
      patchShelf.mockResolvedValueOnce(finishedItem());
      await readButton(wrapper).trigger("click");
      await flushPromises();
      expect(patchShelf).toHaveBeenCalledWith(30, { status: "finished", position: 0 });
      const finish = useFinish();
      expect(finish.item).toMatchObject({ id: 30, status: "finished" });
      expect(finish.clubSlug).toBe("readers-ab12");
      expect(useToast().items.map((row) => row.message)).not.toContain("Moved to Finished");

      book.mockResolvedValue(detail({ club_picks: [readers], club_rating: 5, rating_count: 1 }));
      finish.close(finishedItem({ rating: 5, take: "Yes." }));
      await flushPromises();
      expect(wrapper.findAll(".star-row .star-btn")[4].attributes("aria-pressed")).toBe("true");
      expect((wrapper.get(".take-row textarea").element as HTMLTextAreaElement).value).toBe("Yes.");
      // The club's average is read again after the moment's save.
      expect(book).toHaveBeenCalledTimes(2);
    });

    it("opens no moment after a 409 whose entry was finished already", async () => {
      addToShelf.mockRejectedValue(new ApiError("Already on your shelf", 409, "", { item: finishedItem({ rating: 3 }) }));
      const { wrapper } = await mountBook();
      await readButton(wrapper).trigger("click");
      await flushPromises();
      expect(patchShelf).not.toHaveBeenCalled();
      expect(useFinish().item).toBeNull();
      expect(useToast().items).toHaveLength(0);
      expect(wrapper.findAll(".star-row .star-btn")[2].attributes("aria-pressed")).toBe("true");
    });

    it("opens the moment after a 409 whose entry was still being read", async () => {
      addToShelf.mockRejectedValue(
        new ApiError("Already on your shelf", 409, "", { item: finishedItem({ status: "currently_reading", progress: 40 }) }),
      );
      patchShelf.mockResolvedValue(finishedItem());
      const { wrapper } = await mountBook();
      await readButton(wrapper).trigger("click");
      await flushPromises();
      expect(patchShelf).toHaveBeenCalledWith(30, { status: "finished", position: 0 });
      expect(useFinish().item).toMatchObject({ id: 30, status: "finished" });
      expect(useFinish().clubSlug).toBeNull();
    });

    it("keeps the toast for any other status", async () => {
      book.mockResolvedValue(detail({ shelf_id: 30, on_shelf: "finished", rating: 4 }));
      const { wrapper } = await mountBook();
      patchShelf.mockResolvedValue(finishedItem({ status: "currently_reading", rating: null }));
      await wrapper.findAll(".status-control button").find((node) => node.text() === "Reading")!.trigger("click");
      await flushPromises();
      expect(useFinish().item).toBeNull();
      expect(useToast().items.at(-1)?.message).toBe("Moved to Reading");
    });

    it("hands the progress sheet its entry and adopts the finish it reports, without a progress Undo", async () => {
      book.mockResolvedValue(detail({ club_picks: [readers], shelf_id: 30, on_shelf: "currently_reading", progress: 60 }));
      const { wrapper } = await mountBook();
      const editor = wrapper.getComponent(ProgressEditor);
      expect(editor.props("shelfId")).toBe(30);
      expect(editor.props("clubSlug")).toBe("readers-ab12");
      // An earlier progress save left an Undo; the finish takes it away.
      patchShelf.mockResolvedValueOnce(shelfItem({ id: 30, status: "currently_reading", progress: 90, book: { id: 4 } }));
      editor.vm.$emit("save", 90);
      await flushPromises();
      expect(useToast().items.at(-1)?.action?.label).toBe("Undo");
      editor.vm.$emit("finished", finishedItem());
      await flushPromises();
      expect(useToast().items.some((row) => row.action)).toBe(false);
      expect(wrapper.find(".star-row").exists()).toBe(true);
      expect(wrapper.findComponent(ProgressEditor).exists()).toBe(false);
    });

    it("keeps the rating on a second tap of its star and clears it with “Remove rating”", async () => {
      book.mockResolvedValue(detail({ shelf_id: 30, on_shelf: "finished", rating: 4 }));
      const { wrapper } = await mountBook();
      await wrapper.findAll(".star-row .star-btn")[3].trigger("click");
      await flushPromises();
      expect(patchShelf).not.toHaveBeenCalled();
      patchShelf.mockResolvedValue(finishedItem({ rating: null }));
      await button(wrapper, "Remove rating")!.trigger("click");
      await flushPromises();
      expect(patchShelf).toHaveBeenCalledWith(30, { rating: null });
      expect(useToast().items.at(-1)?.message).toBe("Rating cleared");
      expect(button(wrapper, "Remove rating")).toBeUndefined();
    });
  });
  describe("the reader's own cover", () => {
    const ON_SHELF = { shelf_id: 30, on_shelf: "want_to_read" as const, cover_url: "/media/std/l", cover_thumb_url: "/media/std/s", cover_public_id: "std" };
    const entry = (publicId: string | null, personal: boolean, id = 30) =>
      shelfItem({
        id,
        personal_cover: personal,
        book: {
          id: 4,
          cover_url: publicId ? `/media/${publicId}/l` : null,
          cover_thumb_url: publicId ? `/media/${publicId}/s` : null,
          cover_public_id: publicId,
        },
      });
    const heroSrc = (wrapper: Page) => wrapper.get(".hero-cover img").attributes("src");

    async function openChooser(wrapper: Page) {
      await openMore(wrapper);
      await wrapper.get(".book-actions .choose-cover").trigger("click");
      await flushPromises();
    }

    it("chooses a cover from the … sheet, shows it in the hero, and Undo brings the standard one back", async () => {
      book.mockResolvedValue(detail(ON_SHELF));
      coverCandidates.mockResolvedValue({
        partial: false,
        items: [
          { key: "media:std", source: "catalog", cover_url: "/media/std/l", cover_thumb_url: "/media/std/s", cover_public_id: "std", edition_label: null, chosen: true },
          { key: "ol:2", source: "openlibrary", cover_url: "/media/ol2/l", cover_thumb_url: "/media/ol2/s", cover_public_id: "ol2", edition_label: { publisher: "Diogenes", year: 2019 }, chosen: false },
        ],
      });
      const { wrapper } = await mountBook();
      expect(heroSrc(wrapper)).toBe("/media/std/l");
      await openChooser(wrapper);
      // One sheet at a time: the "…" sheet made way for the chooser.
      expect(wrapper.findAll(".sheet")).toHaveLength(1);
      expect(wrapper.get(".sheet").attributes("aria-label")).toBe("Choose cover");
      expect(coverCandidates).toHaveBeenCalledWith(4, expect.anything());

      chooseCover.mockResolvedValue(entry("ol2", true));
      await wrapper.findAll(".cover-tile")[1]!.trigger("click");
      await vi.waitFor(() => expect(wrapper.find(".cover-chooser").exists()).toBe(false));
      expect(chooseCover).toHaveBeenCalledWith(30, "ol:2");
      expect(heroSrc(wrapper)).toBe("/media/ol2/l");
      const toast = useToast().items.at(-1)!;
      expect(toast.message).toBe("Cover changed");

      clearPersonalCover.mockResolvedValue(entry("std", false));
      await useToast().run(toast.id);
      await flushPromises();
      expect(clearPersonalCover).toHaveBeenCalledWith(30);
      expect(heroSrc(wrapper)).toBe("/media/std/l");
    });

    it("uploads the reader's own photo after the chooser closed and shows it", async () => {
      book.mockResolvedValue(detail(ON_SHELF));
      coverCandidates.mockResolvedValue({ partial: false, items: [] });
      uploadPersonalCover.mockResolvedValue(entry("mine", true));
      const { wrapper } = await mountBook();
      await openChooser(wrapper);
      const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
      await choose(wrapper, ".own-photo", photo);
      expect(wrapper.find(".cover-chooser").exists()).toBe(false);
      expect(uploadPersonalCover).toHaveBeenCalledWith(30, photo);
      expect(uploadCover).not.toHaveBeenCalled();
      expect(heroSrc(wrapper)).toBe("/media/mine/l");
      expect(useToast().items.at(-1)?.message).toBe("Cover changed");
    });

    it("does not offer the choice for a book off the shelf", async () => {
      book.mockResolvedValue(detail({ cover_url: "/media/std/l", cover_thumb_url: "/media/std/s", cover_public_id: "std" }));
      const { wrapper } = await mountBook();
      await openMore(wrapper);
      expect(wrapper.find(".choose-cover").exists()).toBe(false);
    });

    it("keeps the own cover in the hero when a photo becomes the book's cover for everyone", async () => {
      book.mockResolvedValue(detail({ ...ON_SHELF, cover_url: "/media/mine/l", cover_thumb_url: "/media/mine/s", cover_public_id: "mine", personal_cover: true }));
      adminSetCover.mockResolvedValue({ book: bookOut({ id: 4, cover_url: "/media/adm/l", cover_thumb_url: "/media/adm/s", cover_public_id: "adm" }) });
      const { wrapper } = await mountBook("/book/4", [], true);
      await openMore(wrapper);
      await choose(wrapper, ".admin-set-cover", new File(["x"], "cover.png", { type: "image/png" }));
      expect(adminSetCover).toHaveBeenCalled();
      expect(heroSrc(wrapper)).toBe("/media/mine/l");
    });

    it("removes the book with its own cover, and Undo chooses that cover again", async () => {
      book.mockResolvedValue(detail({ ...ON_SHELF, cover_url: "/media/mine/l", cover_thumb_url: "/media/mine/s", cover_public_id: "mine", personal_cover: true }));
      const { wrapper } = await mountBook();
      removeFromShelf.mockResolvedValue(undefined);
      book.mockResolvedValue(detail({ ...ON_SHELF, shelf_id: null, on_shelf: null }));
      await wrapper.findAll("button").find((node) => node.text() === "Remove")!.trigger("click");
      await flushPromises();
      expect(removeFromShelf).toHaveBeenCalledWith(30);
      // Off the shelf, the page shows the cover everyone sees.
      expect(heroSrc(wrapper)).toBe("/media/std/l");

      addToShelf.mockResolvedValue(entry("std", false, 31));
      chooseCover.mockResolvedValue(entry("mine", true, 31));
      await useToast().run(useToast().items.at(-1)!.id);
      await flushPromises();
      expect(addToShelf).toHaveBeenCalledWith(expect.objectContaining({ work_id: 4, status: "want_to_read" }));
      expect(chooseCover).toHaveBeenCalledWith(31, "media:mine");
      expect(heroSrc(wrapper)).toBe("/media/mine/l");
    });

    it("re-adds a book without its own cover without choosing one", async () => {
      book.mockResolvedValue(detail(ON_SHELF));
      const { wrapper } = await mountBook();
      removeFromShelf.mockResolvedValue(undefined);
      await wrapper.findAll("button").find((node) => node.text() === "Remove")!.trigger("click");
      await flushPromises();
      addToShelf.mockResolvedValue(entry("std", false, 31));
      await useToast().run(useToast().items.at(-1)!.id);
      await flushPromises();
      expect(addToShelf).toHaveBeenCalled();
      expect(chooseCover).not.toHaveBeenCalled();
    });
  });
});
