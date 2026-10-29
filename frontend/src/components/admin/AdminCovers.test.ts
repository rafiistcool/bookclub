import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoverSuggestionOut, MissingCoverOut } from "../../api/generated";
import { bookOut, user } from "../../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  adminCoverSuggestionPage: vi.fn(),
  approveCoverSuggestion: vi.fn(),
  rejectCoverSuggestion: vi.fn(),
  adminMissingCovers: vi.fn(),
  adminSearchCover: vi.fn(),
  adminSetCover: vi.fn(),
};

vi.mock("../../api/client", () => {
  class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status: number, detail: unknown = null) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../../api/client";
import { i18n } from "../../i18n";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import AdminCovers from "./AdminCovers.vue";

/** An open suggestion for a legacy book without a cover, from a German edition on Open Library. */
function suggestion(overrides: Partial<CoverSuggestionOut> = {}): CoverSuggestionOut {
  return {
    id: 5,
    status: "open",
    work_id: 40,
    book: bookOut({ id: 40, title: "Die Tribute von Panem 1", authors: "Suzanne Collins", source: "legacy" }),
    cover_url: "/media/candcandcandcandcandcandca/l",
    cover_thumb_url: "/media/candcandcandcandcandcandca/s",
    cover_public_id: "candcandcandcandcandcandca",
    ol_work_id: "OL5735363W",
    ol_edition_id: "OL24945219M",
    title: "Die Tribute von Panem: Tödliche Spiele",
    authors: "Suzanne Collins",
    year: 2009,
    language: "ger",
    reason: "number_differs",
    created_at: "2026-10-29T08:00:00Z",
    decided_at: null,
    ...overrides,
  };
}

function missingRow(overrides: Partial<MissingCoverOut> = {}): MissingCoverOut {
  return {
    book: bookOut({ id: 41, title: "Der Distelfink", authors: "Donna Tartt", source: "legacy" }),
    in_use: true,
    imported: true,
    state: "none",
    failing_edition_id: null,
    failed_at: null,
    searched_at: null,
    outcome: null,
    next_search_at: null,
    failures: 0,
    after_takedown: false,
    suggestion_id: null,
    ...overrides,
  };
}

function page<T>(items: T[], hasMore = false, number = 1) {
  return { items, page: number, has_more: hasMore };
}

async function mountCovers(counts = { suggestions: 1, missing: 2 }) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ username: "root", is_admin: true, timezone: "UTC" });
  useLocale().locale = i18n.global.locale.value;
  const wrapper = mount(AdminCovers, { props: { counts }, global: { plugins: [pinia] } });
  await flushPromises();
  return wrapper;
}

function buttonIn(scope: Pick<VueWrapper, "findAll">, label: string) {
  return scope.findAll("button").find((node) => node.text() === label)!;
}

async function showMissing(wrapper: VueWrapper) {
  await buttonIn(wrapper.get(".cover-sections"), "Without a cover 2").trigger("click");
  await flushPromises();
}

async function pickFile(scope: Pick<VueWrapper, "get">, file: File) {
  const input = scope.get("input[type=file]");
  Object.defineProperty(input.element, "files", { value: [file], configurable: true });
  await input.trigger("change");
  await flushPromises();
}

describe("AdminCovers", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.adminCoverSuggestionPage.mockResolvedValue(page([]));
    api.adminMissingCovers.mockResolvedValue(page([]));
    i18n.global.locale.value = "en";
  });

  it("shows our book beside Open Library's candidate, with its picture from our own /media", async () => {
    api.adminCoverSuggestionPage.mockResolvedValue(page([suggestion()]));
    const wrapper = await mountCovers();
    expect(api.adminCoverSuggestionPage).toHaveBeenCalledWith("open", 1);
    expect(api.adminMissingCovers).not.toHaveBeenCalled();
    expect(wrapper.get(".cover-sections").text()).toContain("Suggestions 1");
    const item = wrapper.get(".suggestion-item");
    expect(item.get(".suggestion-reason").text()).toBe("A number in the title differs: maybe another volume.");
    const [ours, theirs] = item.findAll(".compare-side");
    expect(ours!.text()).toContain("Our book");
    expect(ours!.text()).toContain("Die Tribute von Panem 1");
    expect(ours!.get("a").attributes("href")).toBe("/book/40");
    expect(theirs!.text()).toContain("Die Tribute von Panem: Tödliche Spiele");
    expect(theirs!.text()).toContain("2009 · German");
    // The thumbnail first, as every small cover; never a picture from another host.
    expect(theirs!.get("img").attributes("src")).toBe("/media/candcandcandcandcandcandca/s");
    const link = theirs!.get("a");
    expect(link.text()).toBe("View on Open Library");
    expect(link.attributes("href")).toBe("https://openlibrary.org/books/OL24945219M");
    expect(link.attributes("target")).toBe("_blank");
    expect(link.attributes("rel")).toBe("noreferrer");
  });

  it("uses a cover only after a second tap", async () => {
    api.adminCoverSuggestionPage.mockResolvedValue(page([suggestion(), suggestion({ id: 6, work_id: 42 })]));
    api.approveCoverSuggestion.mockResolvedValue(suggestion({ status: "approved", decided_at: "2026-10-29T09:00:00Z" }));
    const wrapper = await mountCovers();
    const first = wrapper.findAll(".suggestion-item")[0]!;
    await buttonIn(first, "Use this cover").trigger("click");
    expect(api.approveCoverSuggestion).not.toHaveBeenCalled();
    expect(first.get(".confirm-blurb").text()).toContain("for everyone at once");
    await buttonIn(first, "Use it for everyone").trigger("click");
    await flushPromises();
    expect(api.approveCoverSuggestion).toHaveBeenCalledWith(5);
    expect(wrapper.findAll(".suggestion-item")).toHaveLength(1);
    expect(useToast().items.at(-1)?.message).toBe("The cover is set");
    expect(wrapper.emitted("changed")).toHaveLength(1);
  });

  it("rejects for good after a second tap, and switches or cancels the confirmation", async () => {
    api.adminCoverSuggestionPage.mockResolvedValue(page([suggestion()]));
    api.rejectCoverSuggestion.mockResolvedValue(suggestion({ status: "rejected", decided_at: "2026-10-29T09:00:00Z" }));
    const wrapper = await mountCovers();
    const item = wrapper.get(".suggestion-item");
    await buttonIn(item, "Use this cover").trigger("click");
    await buttonIn(item, "Cancel").trigger("click");
    expect(item.find(".confirm-blurb").exists()).toBe(false);
    await buttonIn(item, "Use this cover").trigger("click");
    // The other decision takes the confirmation over.
    await buttonIn(item, "Not this book").trigger("click");
    expect(item.get(".confirm-blurb").text()).toContain("never suggested for this book again");
    expect(api.rejectCoverSuggestion).not.toHaveBeenCalled();
    await buttonIn(item, "Not this book, for good").trigger("click");
    await flushPromises();
    expect(api.approveCoverSuggestion).not.toHaveBeenCalled();
    expect(api.rejectCoverSuggestion).toHaveBeenCalledWith(5);
    expect(wrapper.find(".suggestion-item").exists()).toBe(false);
    expect(wrapper.text()).toContain("No suggestion waits for you.");
    expect(useToast().items.at(-1)?.message).toBe("It won't be suggested again");
  });

  it("reads the list again when the book got a cover meanwhile (409)", async () => {
    api.adminCoverSuggestionPage.mockResolvedValueOnce(page([suggestion()]));
    api.adminCoverSuggestionPage.mockResolvedValueOnce(page([]));
    api.approveCoverSuggestion.mockRejectedValue(
      new ApiError("This book has a cover now.", 409, "This book has a cover now"),
    );
    const wrapper = await mountCovers();
    const item = wrapper.get(".suggestion-item");
    await buttonIn(item, "Use this cover").trigger("click");
    await buttonIn(item, "Use it for everyone").trigger("click");
    await flushPromises();
    expect(api.adminCoverSuggestionPage).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".suggestion-item").exists()).toBe(false);
    expect(useToast().items.at(-1)?.message).toBe("This book has a cover now.");
    expect(wrapper.emitted("changed")).toHaveLength(1);
  });

  it("offers no use of a picture that went offline, and lists decided suggestions with their decision", async () => {
    api.adminCoverSuggestionPage.mockResolvedValueOnce(
      page([suggestion({ cover_url: null, cover_thumb_url: null, cover_public_id: null, reason: "after_takedown" })]),
    );
    const wrapper = await mountCovers();
    const item = wrapper.get(".suggestion-item");
    expect(item.text()).toContain("The picture is no longer online");
    expect(buttonIn(item, "Use this cover").attributes("disabled")).toBeDefined();
    expect(buttonIn(item, "Not this book").attributes("disabled")).toBeUndefined();

    api.adminCoverSuggestionPage.mockResolvedValueOnce(
      page([
        suggestion({ id: 7, status: "approved", decided_at: "2026-10-29T10:00:00Z" }),
        suggestion({ id: 8, status: "closed", decided_at: "2026-10-29T11:00:00Z", book: null, ol_edition_id: null }),
      ]),
    );
    await buttonIn(wrapper.get(".queue-views"), "Decided").trigger("click");
    await flushPromises();
    expect(api.adminCoverSuggestionPage).toHaveBeenLastCalledWith("resolved", 1);
    const [used, closed] = wrapper.findAll(".suggestion-item");
    expect(used!.get(".badge").text()).toBe("Used");
    expect(used!.get(".decision").text()).toMatch(/^Used as the cover · /);
    expect(used!.find(".btn-row").exists()).toBe(false);
    expect(closed!.get(".badge").text()).toBe("Closed");
    expect(closed!.text()).toContain("This book is no longer in the catalog.");
    expect(closed!.get(".candidate a").attributes("href")).toBe("https://openlibrary.org/works/OL5735363W");
  });

  it("lists the books without a cover with their state, and switches to all of them", async () => {
    api.adminMissingCovers.mockResolvedValueOnce(
      page(
        [
          missingRow(),
          missingRow({
            book: bookOut({ id: 43, title: "Momo", source: "dnb" }),
            imported: false,
            state: "failing",
            failing_edition_id: 13,
            failed_at: "2026-10-28T03:00:00Z",
            searched_at: "2026-10-20T08:00:00Z",
            outcome: "no_match",
            next_search_at: "2099-11-19T08:00:00Z",
          }),
        ],
        true,
      ),
    );
    const wrapper = await mountCovers();
    await showMissing(wrapper);
    expect(api.adminMissingCovers).toHaveBeenCalledWith("in_use", 1);
    const [distelfink, momo] = wrapper.findAll(".missing-item");
    expect(distelfink!.text()).toContain("Der Distelfink");
    expect(distelfink!.findAll(".badge").map((node) => node.text())).toEqual(["Readers use it", "Imported"]);
    expect(distelfink!.findAll(".cover-notes li").map((node) => node.text())).toEqual(["Not searched yet."]);
    expect(distelfink!.get("a.btn").attributes("href")).toBe("/book/41");
    expect(momo!.findAll(".cover-notes li").map((node) => node.text())).toEqual([
      "Open Library's picture of edition 13 failed to load on 28 Oct 2026.",
      "No match on Open Library on 20 Oct 2026.",
      "Next try on 19 Nov 2099.",
    ]);
    expect(momo!.get("label.cover-photo").text()).toBe("Upload for edition 13");

    api.adminMissingCovers.mockResolvedValueOnce(page([missingRow({ book: bookOut({ id: 44, title: "Gedichte" }), in_use: false })], false, 2));
    await buttonIn(wrapper, "Show more").trigger("click");
    await flushPromises();
    expect(api.adminMissingCovers).toHaveBeenLastCalledWith("in_use", 1);
    expect(wrapper.findAll(".missing-item")).toHaveLength(3);

    api.adminMissingCovers.mockResolvedValueOnce(page([]));
    await buttonIn(wrapper, "All").trigger("click");
    await flushPromises();
    expect(api.adminMissingCovers).toHaveBeenLastCalledWith("all", 1);
    expect(wrapper.text()).toContain("Every book here has a cover.");
  });

  it("searches a book now: a found cover leaves the list, a suggestion and a miss are noted", async () => {
    api.adminMissingCovers.mockResolvedValue(
      page([
        missingRow(),
        missingRow({ book: bookOut({ id: 42, title: "Circe" }) }),
        missingRow({ book: bookOut({ id: 43, title: "Momo" }), failures: 2 }),
      ]),
    );
    const wrapper = await mountCovers();
    await showMissing(wrapper);
    const rows = () => wrapper.findAll(".missing-item");

    api.adminSearchCover.mockResolvedValueOnce({ outcome: "attached", book: bookOut({ id: 41 }), suggestion: null });
    await buttonIn(rows()[0]!, "Search now").trigger("click");
    await flushPromises();
    expect(api.adminSearchCover).toHaveBeenLastCalledWith(41);
    expect(useToast().items.at(-1)?.message).toBe("Found it: Open Library's cover is the book's cover now.");
    expect(rows().map((row) => row.get("strong").text())).toEqual(["Circe", "Momo"]);
    expect(wrapper.emitted("changed")).toHaveLength(1);

    api.adminSearchCover.mockResolvedValueOnce({
      outcome: "suggested",
      book: bookOut({ id: 42 }),
      suggestion: suggestion({ id: 9, work_id: 42 }),
    });
    await buttonIn(rows()[0]!, "Search now").trigger("click");
    await flushPromises();
    expect(useToast().items.at(-1)?.message).toBe("Found a possible cover. It waits under Suggestions.");
    expect(rows()[0]!.get(".cover-notes").text()).toBe("A suggestion waits under Suggestions.");
    expect(wrapper.emitted("changed")).toHaveLength(2);

    api.adminSearchCover.mockResolvedValueOnce({ outcome: "no_match", book: bookOut({ id: 43 }), suggestion: null });
    await buttonIn(rows()[1]!, "Search now").trigger("click");
    await flushPromises();
    expect(useToast().items.at(-1)?.message).toBe("No match on Open Library.");
    expect(rows()[1]!.get(".cover-notes").text()).toMatch(/^No match on Open Library on /);
    expect(rows()).toHaveLength(2);

    // The suggestion it found shows when the suggestions are shown again.
    api.adminCoverSuggestionPage.mockResolvedValueOnce(page([suggestion({ id: 9, work_id: 42 })]));
    await buttonIn(wrapper.get(".cover-sections"), "Suggestions 1").trigger("click");
    await flushPromises();
    expect(api.adminCoverSuggestionPage).toHaveBeenCalledTimes(2);
    expect(wrapper.findAll(".suggestion-item")).toHaveLength(1);
  });

  it("drops a book that has a cover by now, and keeps it when Open Library does not answer", async () => {
    api.adminMissingCovers.mockResolvedValue(page([missingRow(), missingRow({ book: bookOut({ id: 42, title: "Circe" }) })]));
    const wrapper = await mountCovers();
    await showMissing(wrapper);

    api.adminSearchCover.mockRejectedValueOnce(new ApiError("This book has a cover now.", 409, "This book has a cover now"));
    await buttonIn(wrapper.findAll(".missing-item")[0]!, "Search now").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".missing-item")).toHaveLength(1);
    expect(useToast().items.at(-1)?.message).toBe("This book has a cover now.");

    api.adminSearchCover.mockRejectedValueOnce(
      new ApiError("Could not search the library right now. Try again.", 502, "Could not search the library right now. Try again."),
    );
    await buttonIn(wrapper.get(".missing-item"), "Search now").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".missing-item")).toHaveLength(1);
    expect(useToast().items.at(-1)?.message).toBe("Could not search the library right now. Try again.");
  });

  it("uploads a picture as the cover of the book, or of the edition whose picture fails", async () => {
    api.adminMissingCovers.mockResolvedValue(
      page([
        missingRow(),
        missingRow({ book: bookOut({ id: 43, title: "Momo" }), state: "failing", failing_edition_id: 13, failed_at: "2026-10-28T03:00:00Z" }),
      ]),
    );
    const wrapper = await mountCovers();
    await showMissing(wrapper);
    const picture = new File(["x"], "cover.jpg", { type: "image/jpeg" });

    api.adminSetCover.mockResolvedValueOnce({ book: bookOut({ id: 43, cover_url: "/media/up/l" }) });
    await pickFile(wrapper.findAll(".missing-item")[1]!, picture);
    expect(api.adminSetCover).toHaveBeenLastCalledWith(43, picture, 13);

    api.adminSetCover.mockResolvedValueOnce({ book: bookOut({ id: 41, cover_url: "/media/up2/l" }) });
    const first = wrapper.get(".missing-item");
    expect(first.get("label.cover-photo").text()).toBe("Upload a picture");
    await pickFile(first, picture);
    expect(api.adminSetCover).toHaveBeenLastCalledWith(41, picture, null);
    expect(wrapper.find(".missing-item").exists()).toBe(false);
    expect(useToast().items.at(-1)?.message).toBe("The cover is set for everyone");
    expect(wrapper.emitted("changed")).toHaveLength(2);
  });

  it("speaks German", async () => {
    i18n.global.locale.value = "de";
    api.adminCoverSuggestionPage.mockResolvedValue(page([suggestion()]));
    const wrapper = await mountCovers();
    expect(wrapper.get("h2").text()).toBe("Cover");
    const item = wrapper.get(".suggestion-item");
    expect(buttonIn(item, "Dieses Cover nehmen")).toBeTruthy();
    expect(buttonIn(item, "Nicht dieses Buch")).toBeTruthy();
    expect(item.text()).toContain("2009 · Deutsch");
  });
});
