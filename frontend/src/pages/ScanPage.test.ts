import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { edition, isbnHit, searchHit, shelfItem } from "../test/fixtures";

const lookupIsbn = vi.fn();
const addToShelf = vi.fn();
const patchShelf = vi.fn();
const search = vi.fn();
const addEdition = vi.fn();
const createCustomBook = vi.fn();
const push = vi.fn();
const back = vi.fn();

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api/client")>()),
  api: {
    lookupIsbn: (...args: unknown[]) => lookupIsbn(...args),
    addToShelf: (...args: unknown[]) => addToShelf(...args),
    patchShelf: (...args: unknown[]) => patchShelf(...args),
    search: (...args: unknown[]) => search(...args),
    addEdition: (...args: unknown[]) => addEdition(...args),
    createCustomBook: (...args: unknown[]) => createCustomBook(...args),
  },
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push, back }),
}));

import { ApiError } from "../api/client";
import ScanPage from "./ScanPage.vue";

const ISBN = "9780316769488";

/** The real scanner needs a camera; this one reads an ISBN on a click. */
const scannerStub = {
  name: "BarcodeScanner",
  emits: ["isbn", "cancel"],
  template: `<div class="scanner-stub"><button type="button" class="scan-stub" @click="$emit('isbn', '${ISBN}')">scan</button></div>`,
};

let wrapper: VueWrapper | null = null;

async function mountPage() {
  wrapper = mount(ScanPage, {
    attachTo: document.body,
    global: { plugins: [createPinia()], stubs: { BarcodeScanner: scannerStub } },
  });
  await flushPromises();
  return wrapper;
}

async function scan(w: VueWrapper) {
  await w.get(".scan-stub").trigger("click");
  await flushPromises();
}

function button(w: VueWrapper, label: string) {
  const found = w.findAll("button").find((candidate) => candidate.text() === label);
  if (!found) throw new Error(`no button "${label}" in: ${w.text()}`);
  return found;
}

function apiError(status: number, detail: string, body: Record<string, unknown> = {}) {
  return new ApiError(detail, status, detail, { detail, ...body });
}

describe("ScanPage", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const mock of [lookupIsbn, addToShelf, patchShelf, search, addEdition, createCustomBook, push, back]) {
      mock.mockReset();
    }
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    vi.useRealTimers();
  });

  it("looks a scanned ISBN up once and adds the hit with its edition", async () => {
    lookupIsbn.mockResolvedValue(isbnHit());
    addToShelf.mockResolvedValue(shelfItem({ id: 5, status: "want_to_read" }));
    const w = await mountPage();
    await scan(w);
    expect(lookupIsbn).toHaveBeenCalledTimes(1);
    expect(lookupIsbn.mock.calls[0][0]).toBe(ISBN);
    expect(lookupIsbn.mock.calls[0][1]).toBeInstanceOf(AbortSignal);
    expect(w.find(".scanner-stub").exists()).toBe(false);
    expect(w.text()).toContain("The Catcher in the Rye");
    expect(w.text()).toContain("J. D. Salinger");
    expect(w.text()).toContain("1951 · 277 pages · ISBN 9780316769488");
    expect(addToShelf).not.toHaveBeenCalled();

    await button(w, "Want").trigger("click");
    await flushPromises();
    expect(addToShelf).toHaveBeenCalledWith({ work_id: 9, edition_id: 40, status: "want_to_read" });
    expect(w.text()).toContain("On your shelf: Want to read");
    expect(w.text()).toContain("This edition is on your shelf.");
    expect(w.find("a[href='/book/9']").text()).toBe("Open book");
  });

  it("sets the edition on the entry when the book is on the shelf already", async () => {
    lookupIsbn.mockResolvedValue(isbnHit({ on_shelf: true, shelf_id: 7 }));
    patchShelf.mockResolvedValue(shelfItem({ id: 7 }));
    const w = await mountPage();
    await scan(w);
    expect(w.text()).toContain("On your shelf");
    expect(w.findAll(".segmented button")).toHaveLength(0);
    await button(w, "Use this edition").trigger("click");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(7, { edition_id: 40 });
    expect(addToShelf).not.toHaveBeenCalled();
    expect(w.text()).toContain("This edition is on your shelf.");
  });

  it("turns a 409 on add into a PATCH of the entry that is there", async () => {
    lookupIsbn.mockResolvedValue(isbnHit());
    const existing = shelfItem({ id: 12, status: "want_to_read" });
    addToShelf.mockRejectedValue(apiError(409, "Already on your shelf", { item: existing }));
    patchShelf.mockResolvedValue(shelfItem({ id: 12, status: "finished" }));
    const w = await mountPage();
    await scan(w);
    await button(w, "Done").trigger("click");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(12, { edition_id: 40, status: "finished", position: 0 });
    expect(w.text()).toContain("On your shelf: Finished");
  });

  it("offers a photo when the edition has no cover, and not when it has one", async () => {
    lookupIsbn.mockResolvedValueOnce(isbnHit()).mockResolvedValueOnce(isbnHit({ cover_url: "/media/c/l.webp" }));
    const w = await mountPage();
    await scan(w);
    expect(w.find(".cover-photo").text()).toBe("Add a photo of the cover");
    await button(w, "Scan another").trigger("click");
    await scan(w);
    expect(w.find(".cover-photo").exists()).toBe(false);
  });

  it("finds the work by title on a miss and adds the ISBN as its edition, then shows the hit", async () => {
    lookupIsbn
      .mockRejectedValueOnce(apiError(404, "No book found for that ISBN"))
      .mockResolvedValueOnce(isbnHit({ work_id: 3, edition_id: 77, title: "Circe" }));
    search.mockResolvedValue({ items: [searchHit({ id: 3, title: "Circe" })], page: 1, has_more: false });
    addEdition.mockResolvedValue(edition({ id: 77, work_id: 3, isbn13: ISBN }));
    const w = await mountPage();
    await scan(w);
    expect(w.text()).toContain("This ISBN is not in the catalog yet");
    expect(w.text()).toContain(`ISBN ${ISBN}`);
    const order = w.findAll("button").map((candidate) => candidate.text());
    expect(order.indexOf("Find the book by title")).toBeLessThan(order.indexOf("Add it as a new book"));

    await button(w, "Find the book by title").trigger("click");
    await w.get("input[type='search']").setValue("circe miller");
    await w.get("form[role='search']").trigger("submit");
    await flushPromises();
    expect(search).toHaveBeenCalledWith({ q: "circe miller", limit: 10 });
    await w.get(".work-row").trigger("click");
    const isbnField = w.get<HTMLInputElement>("input[name='isbn']");
    expect(isbnField.element.value).toBe(ISBN);
    expect(w.text()).toContain("The ISBN you looked up.");

    await w.get("form.add-edition").trigger("submit");
    await flushPromises();
    expect(addEdition).toHaveBeenCalledWith(3, expect.objectContaining({ isbn: ISBN }));
    expect(lookupIsbn).toHaveBeenCalledTimes(2);
    expect(lookupIsbn.mock.calls[1][0]).toBe(ISBN);
    expect(w.text()).toContain("Circe");
    expect(w.find(".cover-photo").text()).toBe("Add a photo of the cover");
  });

  it("adds a new book with the ISBN on a miss, then shows the hit with the photo button", async () => {
    lookupIsbn
      .mockRejectedValueOnce(apiError(404, "No book found for that ISBN"))
      .mockResolvedValueOnce(isbnHit({ work_id: 31, edition_id: 90, title: "Kitchen Zine 2", authors: "Ada" }));
    createCustomBook.mockResolvedValue({ id: 31, title: "Kitchen Zine 2" });
    const w = await mountPage();
    await scan(w);
    await button(w, "Add it as a new book").trigger("click");
    const sheet = w.get(".sheet");
    const fields = sheet.findAll<HTMLInputElement>("input[type='text']");
    expect(fields[0].element.value).toBe("");
    expect(fields[2].element.value).toBe(ISBN);
    await fields[0].setValue("Kitchen Zine 2");
    await sheet.get("form").trigger("submit");
    await flushPromises();
    expect(createCustomBook).toHaveBeenCalledWith(expect.objectContaining({ title: "Kitchen Zine 2", isbn: ISBN }));
    expect(push).not.toHaveBeenCalled();
    expect(w.find(".sheet").exists()).toBe(false);
    expect(lookupIsbn).toHaveBeenCalledTimes(2);
    expect(w.text()).toContain("Kitchen Zine 2");
    expect(w.find(".cover-photo").text()).toBe("Add a photo of the cover");
  });

  describe("when the reader changes the scanned ISBN in the form", () => {
    /** The ISBN-10 and ISBN-13 of another book than the scanned one. */
    const TYPED = "0-306-40615-2";
    const TYPED_13 = "9780306406157";

    async function missAndChooseCirce(w: VueWrapper) {
      search.mockResolvedValue({ items: [searchHit({ id: 3, title: "Circe" })], page: 1, has_more: false });
      await scan(w);
      await button(w, "Find the book by title").trigger("click");
      await w.get("input[type='search']").setValue("circe");
      await w.get("form[role='search']").trigger("submit");
      await flushPromises();
      await w.get(".work-row").trigger("click");
    }

    async function missAndOpenSheet(w: VueWrapper) {
      await scan(w);
      await button(w, "Add it as a new book").trigger("click");
      const sheet = w.get(".sheet");
      await sheet.findAll<HTMLInputElement>("input[type='text']")[0].setValue("Kitchen Zine 3");
      return sheet;
    }

    it("looks up the ISBN the edition was saved with", async () => {
      lookupIsbn
        .mockRejectedValueOnce(apiError(404, "No book found for that ISBN"))
        .mockResolvedValueOnce(isbnHit({ isbn: TYPED_13, work_id: 3, edition_id: 78, title: "Circe" }));
      addEdition.mockResolvedValue(edition({ id: 78, work_id: 3, isbn13: TYPED_13 }));
      const w = await mountPage();
      await missAndChooseCirce(w);
      await w.get("input[name='isbn']").setValue(TYPED);
      await w.get("form.add-edition").trigger("submit");
      await flushPromises();
      expect(addEdition).toHaveBeenCalledWith(3, expect.objectContaining({ isbn: TYPED }));
      expect(lookupIsbn).toHaveBeenCalledTimes(2);
      expect(lookupIsbn.mock.calls[1][0]).toBe(TYPED_13);
      expect(w.text()).not.toContain("This ISBN is not in the catalog yet");
      expect(w.text()).toContain(`ISBN ${TYPED_13}`);
      expect(w.find(".cover-photo").exists()).toBe(true);
    });

    it("opens the book when the edition was saved without an ISBN", async () => {
      lookupIsbn.mockRejectedValueOnce(apiError(404, "No book found for that ISBN"));
      addEdition.mockResolvedValue(edition({ id: 79, work_id: 3, isbn13: null, publisher: "Kitchen Press" }));
      const w = await mountPage();
      await missAndChooseCirce(w);
      await w.get("input[name='isbn']").setValue("");
      await w.get("input[name='publisher']").setValue("Kitchen Press");
      await w.get("form.add-edition").trigger("submit");
      await flushPromises();
      expect(addEdition).toHaveBeenCalledWith(3, expect.objectContaining({ isbn: null, publisher: "Kitchen Press" }));
      expect(lookupIsbn).toHaveBeenCalledTimes(1);
      expect(push).toHaveBeenCalledWith("/book/3");
    });

    it("looks up the ISBN of the edition chosen after a 409", async () => {
      lookupIsbn
        .mockRejectedValueOnce(apiError(404, "No book found for that ISBN"))
        .mockResolvedValueOnce(isbnHit({ isbn: TYPED_13, work_id: 3, edition_id: 40, title: "Circe" }));
      addEdition.mockRejectedValue(
        apiError(409, "That ISBN is already in the catalog", { work_id: 3, edition_id: 40 }),
      );
      const w = await mountPage();
      await missAndChooseCirce(w);
      await w.get("input[name='isbn']").setValue(TYPED);
      await w.get("form.add-edition").trigger("submit");
      await flushPromises();
      // Typing again after the 409 does not change which edition the choice is.
      await w.get("input[name='isbn']").setValue(ISBN);
      await button(w, "Choose that edition").trigger("click");
      await flushPromises();
      expect(lookupIsbn).toHaveBeenCalledTimes(2);
      expect(lookupIsbn.mock.calls[1][0]).toBe(TYPED_13);
      expect(w.text()).toContain(`ISBN ${TYPED_13}`);
    });

    it("looks up the ISBN the new book was saved with", async () => {
      lookupIsbn
        .mockRejectedValueOnce(apiError(404, "No book found for that ISBN"))
        .mockResolvedValueOnce(isbnHit({ isbn: TYPED_13, work_id: 32, edition_id: 91, title: "Kitchen Zine 3" }));
      createCustomBook.mockResolvedValue({ id: 32, title: "Kitchen Zine 3" });
      const w = await mountPage();
      const sheet = await missAndOpenSheet(w);
      await sheet.findAll<HTMLInputElement>("input[type='text']")[2].setValue(TYPED);
      await sheet.get("form").trigger("submit");
      await flushPromises();
      expect(createCustomBook).toHaveBeenCalledWith(expect.objectContaining({ isbn: TYPED }));
      expect(lookupIsbn).toHaveBeenCalledTimes(2);
      expect(lookupIsbn.mock.calls[1][0]).toBe(TYPED_13);
      expect(w.text()).toContain("Kitchen Zine 3");
      expect(w.text()).toContain(`ISBN ${TYPED_13}`);
    });

    it("opens the new book when it was saved without an ISBN", async () => {
      lookupIsbn.mockRejectedValueOnce(apiError(404, "No book found for that ISBN"));
      createCustomBook.mockResolvedValue({ id: 32, title: "Kitchen Zine 3" });
      const w = await mountPage();
      const sheet = await missAndOpenSheet(w);
      await sheet.findAll<HTMLInputElement>("input[type='text']")[2].setValue("");
      await sheet.get("form").trigger("submit");
      await flushPromises();
      expect(createCustomBook.mock.calls[0][0]).not.toHaveProperty("isbn");
      expect(lookupIsbn).toHaveBeenCalledTimes(1);
      expect(push).toHaveBeenCalledWith("/book/32");
    });
  });

  it("opens the saved book instead of the miss card when its ISBN still misses", async () => {
    lookupIsbn.mockRejectedValue(apiError(404, "No book found for that ISBN"));
    createCustomBook.mockResolvedValue({ id: 33, title: "Kitchen Zine 4" });
    const w = await mountPage();
    await scan(w);
    await button(w, "Add it as a new book").trigger("click");
    const sheet = w.get(".sheet");
    await sheet.findAll<HTMLInputElement>("input[type='text']")[0].setValue("Kitchen Zine 4");
    await sheet.get("form").trigger("submit");
    await flushPromises();
    expect(lookupIsbn).toHaveBeenCalledTimes(2);
    expect(push).toHaveBeenCalledWith("/book/33");
  });

  it.each([
    [429, "Slow down a little. Try again later."],
    [429, "The library is busy. Try again in 20 seconds."],
    [502, "Could not look that ISBN up right now. Try again."],
  ])("keeps the ISBN after a %i and looks it up again", async (status, detail) => {
    lookupIsbn.mockRejectedValueOnce(apiError(status, detail)).mockResolvedValueOnce(isbnHit());
    const w = await mountPage();
    await scan(w);
    expect(w.text()).toContain(detail);
    expect(w.text()).toContain(`ISBN ${ISBN}`);
    expect(w.text()).not.toContain("This ISBN is not in the catalog yet");
    await button(w, "Try again").trigger("click");
    await flushPromises();
    expect(lookupIsbn).toHaveBeenCalledTimes(2);
    expect(lookupIsbn.mock.calls[1][0]).toBe(ISBN);
    expect(w.text()).toContain("The Catcher in the Rye");
  });

  it("shows the seconds after 2 s, and Cancel aborts the lookup and goes back to the camera", async () => {
    vi.useFakeTimers();
    let signal!: AbortSignal;
    lookupIsbn.mockImplementation(
      (_isbn: string, aborted: AbortSignal) =>
        new Promise((_resolve, reject) => {
          signal = aborted;
          aborted.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );
    const w = await mountPage();
    await scan(w);
    expect(w.text()).toContain(`Looking up ISBN ${ISBN}`);
    expect(w.text()).not.toContain("so far");
    await vi.advanceTimersByTimeAsync(2500);
    expect(w.text()).toContain("Still looking… 2s so far.");
    await button(w, "Cancel").trigger("click");
    await flushPromises();
    expect(signal.aborted).toBe(true);
    expect(w.find(".scanner-stub").exists()).toBe(true);
    expect(w.text()).not.toContain("Something went wrong");
  });

  it("leaves for Discover when the scanner is cancelled on a page opened directly", async () => {
    const w = await mountPage();
    w.findComponent(scannerStub).vm.$emit("cancel");
    await flushPromises();
    expect(push).toHaveBeenCalledWith("/discover");
  });
});
