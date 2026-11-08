import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoverCandidateOut } from "../types";
import { shelfItem } from "../test/fixtures";
import { resetScan, scanAnswer } from "../test/coverScan";

vi.mock("../stores/coverScan", () => import("../test/coverScan"));

const coverCandidates = vi.fn();
const chooseCover = vi.fn();
const uploadPersonalCover = vi.fn();
const clearPersonalCover = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  },
  api: {
    coverCandidates: (...args: unknown[]) => coverCandidates(...args),
    chooseCover: (...args: unknown[]) => chooseCover(...args),
    uploadPersonalCover: (...args: unknown[]) => uploadPersonalCover(...args),
    clearPersonalCover: (...args: unknown[]) => clearPersonalCover(...args),
  },
}));

import { ApiError } from "../api/client";
import { COVER_RETRY_MS } from "../covers";
import { i18n } from "../i18n";
import { useToast } from "../stores/toast";
import CoverChooserSheet, { announceCoverChange, sendPersonalCover, shortPublisher } from "./CoverChooserSheet.vue";

function candidate(overrides: Partial<CoverCandidateOut> = {}): CoverCandidateOut {
  const key = overrides.key ?? "ol:1";
  const id = key.replace(/^media:/, "").replace(/\W/g, "");
  return {
    key,
    source: "openlibrary",
    cover_url: `/media/${id}/l`,
    cover_thumb_url: `/media/${id}/s`,
    cover_public_id: id,
    edition_label: null,
    chosen: false,
    ...overrides,
  };
}

function mountSheet(props: Partial<{ personalCover: boolean; coverPublicId: string | null }> = {}) {
  return mount(CoverChooserSheet, {
    props: { entryId: 30, workId: 4, title: "Circe", personalCover: false, coverPublicId: null, ...props },
    attachTo: document.body,
  });
}

type Sheet = ReturnType<typeof mountSheet>;

const tiles = (wrapper: Sheet) => wrapper.findAll(".cover-tile");

describe("CoverChooserSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const mock of [coverCandidates, chooseCover, uploadPersonalCover, clearPersonalCover]) mock.mockReset();
    resetScan();
    i18n.global.locale.value = "de";
  });

  it("shows skeleton tiles while the candidates load, then a grid of their thumbnails", async () => {
    let answer!: (value: unknown) => void;
    coverCandidates.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const wrapper = mountSheet();
    expect(wrapper.get(".sheet").attributes("aria-label")).toBe("Cover wählen");
    expect(coverCandidates).toHaveBeenCalledWith(4, expect.objectContaining({ signal: expect.any(AbortSignal) }));
    expect(wrapper.findAll(".tile-skeleton")).toHaveLength(5);
    // The own photo is there at once.
    expect(wrapper.get(".own-photo").text()).toBe("Eigenes Foto");

    answer({
      partial: false,
      items: [
        candidate({ key: "media:cat", source: "catalog", chosen: true, edition_label: { title: null, publisher: "Diogenes", year: 2019, language: "ger", isbn13: null } }),
        candidate({ key: "ol:2", edition_label: { title: "Circe", publisher: null, year: 2018, language: "eng", isbn13: null } }),
        candidate({ key: "ol:3", edition_label: null }),
        candidate({ key: "ol:5", edition_label: { title: null, publisher: "Kiepenheuer & Witsch GmbH & Co. KG", year: 2021, language: null, isbn13: null } }),
        // Nothing of ours to show: never a tile.
        candidate({ key: "ol:4", cover_url: "https://covers.openlibrary.org/b/id/4-L.jpg", cover_thumb_url: null }),
      ],
    });
    await flushPromises();
    expect(wrapper.find(".tile-skeleton").exists()).toBe(false);
    const shown = tiles(wrapper);
    expect(shown).toHaveLength(4);
    expect(shown.map((tile) => tile.get("img").attributes("src"))).toEqual(["/media/cat/s", "/media/ol2/s", "/media/ol3/s", "/media/ol5/s"]);
    expect(shown.every((tile) => tile.get("img").attributes("loading") === "lazy")).toBe(true);
    // Year and language first, the publisher on its own line: what tells editions apart is never cut off.
    expect(shown.map((tile) => tile.get(".label-facts").text())).toEqual(["2019 · Deutsch", "2018 · Englisch", "Open Library", "2021"]);
    // Each fact is a part of its own, so a narrow tile wraps between them instead of cutting one off.
    expect(shown.map((tile) => tile.findAll(".label-fact").map((part) => part.text()))).toEqual([
      ["2019", "Deutsch"],
      ["2018", "Englisch"],
      ["Open Library"],
      ["2021"],
    ]);
    // The legal form goes, so the name itself keeps its last word.
    expect(shown.map((tile) => tile.find(".label-publisher").exists() ? tile.get(".label-publisher").text() : null)).toEqual([
      "Diogenes",
      null,
      null,
      "Kiepenheuer & Witsch",
    ]);
    // The whole label as the tooltip, with the publisher's full name.
    expect(shown.map((tile) => tile.attributes("title"))).toEqual([
      "2019 · Deutsch · Diogenes", "2018 · Englisch", "Open Library", "2021 · Kiepenheuer & Witsch GmbH & Co. KG",
    ]);
    // The cover the reader sees now is ringed with a check.
    expect(shown.map((tile) => tile.attributes("aria-pressed"))).toEqual(["true", "false", "false", "false"]);
    expect(shown[0]!.find(".tile-check").exists()).toBe(true);
    // No standard row while the standard cover is what the reader sees.
    expect(wrapper.find(".standard-cover").exists()).toBe(false);
    wrapper.unmount();
  });

  it("asks for a failed picture once more after a pause, then the large one, then hides the tile", async () => {
    vi.useFakeTimers();
    try {
      coverCandidates.mockResolvedValue({ partial: true, items: [candidate({ key: "ol:2" }), candidate({ key: "ol:3" })] });
      const wrapper = mountSheet();
      await flushPromises();
      expect(wrapper.text()).toContain("Nicht jede Quelle hat geantwortet");
      const first = tiles(wrapper)[0]!.get("img").element;
      await tiles(wrapper)[0]!.get("img").trigger("error");
      // A busy Open Library (429, 502) answers again later: the tile waits, empty, instead of going.
      expect(tiles(wrapper)).toHaveLength(2);
      expect(tiles(wrapper)[0]!.find("img").exists()).toBe(false);
      await vi.advanceTimersByTimeAsync(COVER_RETRY_MS);
      // The same picture again, in a new element so the browser asks again.
      expect(tiles(wrapper)[0]!.get("img").attributes("src")).toBe("/media/ol2/s");
      expect(tiles(wrapper)[0]!.get("img").element).not.toBe(first);
      await tiles(wrapper)[0]!.get("img").trigger("error");
      expect(tiles(wrapper)[0]!.get("img").attributes("src")).toBe("/media/ol2/l");
      await tiles(wrapper)[0]!.get("img").trigger("error");
      expect(tiles(wrapper)).toHaveLength(1);
      expect(tiles(wrapper)[0]!.get("img").attributes("src")).toBe("/media/ol3/s");
      await tiles(wrapper)[0]!.get("img").trigger("error");
      await vi.advanceTimersByTimeAsync(COVER_RETRY_MS);
      await tiles(wrapper)[0]!.get("img").trigger("error");
      await tiles(wrapper)[0]!.get("img").trigger("error");
      expect(tiles(wrapper)).toHaveLength(0);
      // The own photo always remains.
      expect(wrapper.text()).toContain("Keine weiteren Cover gefunden");
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps a tile whose retry worked, and stops a waiting retry when the sheet goes", async () => {
    vi.useFakeTimers();
    try {
      coverCandidates.mockResolvedValue({ partial: false, items: [candidate({ key: "ol:2" })] });
      const wrapper = mountSheet();
      await flushPromises();
      await tiles(wrapper)[0]!.get("img").trigger("error");
      await vi.advanceTimersByTimeAsync(COVER_RETRY_MS);
      await tiles(wrapper)[0]!.get("img").trigger("load");
      expect(tiles(wrapper)[0]!.get("img").attributes("src")).toBe("/media/ol2/s");
      expect(wrapper.text()).not.toContain("Keine weiteren Cover gefunden");

      // A sheet that goes leaves as many timers behind with a retry waiting as without one.
      const timersLeftBy = async (failing: boolean) => {
        const sheet = mountSheet();
        await flushPromises();
        if (failing) await tiles(sheet)[0]!.get("img").trigger("error");
        const before = vi.getTimerCount() - (failing ? 1 : 0);
        sheet.unmount();
        const left = vi.getTimerCount() - before;
        vi.clearAllTimers();
        return left;
      };
      expect(await timersLeftBy(true)).toBe(await timersLeftBy(false));
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves out a code that names no language", async () => {
    coverCandidates.mockResolvedValue({
      partial: false,
      items: [
        candidate({ key: "ol:2", edition_label: { title: null, publisher: "Diogenes", year: 2019, language: "und", isbn13: null } }),
        candidate({ key: "ol:3", edition_label: { title: null, publisher: null, year: null, language: "mul", isbn13: null } }),
      ],
    });
    const wrapper = mountSheet();
    await flushPromises();
    expect(tiles(wrapper).map((tile) => tile.attributes("title"))).toEqual(["2019 · Diogenes", "Open Library"]);
    expect(wrapper.findAll(".tile-label").map((node) => node.text())).toEqual(["2019Diogenes", "Open Library"]);
    wrapper.unmount();
  });

  it("keeps the own photo's picker closed while a choice saves, and says so for a file picked anyway", async () => {
    coverCandidates.mockResolvedValue({ partial: false, items: [candidate({ key: "ol:2" })] });
    let answer!: (value: unknown) => void;
    chooseCover.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const wrapper = mountSheet();
    await flushPromises();
    const input = wrapper.get(".own-photo input[type=file]");
    expect((input.element as HTMLInputElement).disabled).toBe(false);
    await tiles(wrapper)[0]!.trigger("click");
    expect((input.element as HTMLInputElement).disabled).toBe(true);
    expect(wrapper.get(".own-photo").classes()).toContain("disabled");
    // A picker opened just before the tap still hands its file over: not silently lost.
    const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
    Object.defineProperty(input.element, "files", { value: [photo], configurable: true });
    // (Test utils skip events on a disabled element; the browser's picker does not.)
    input.element.dispatchEvent(new Event("change"));
    await flushPromises();
    expect(wrapper.emitted("picked")).toBeUndefined();
    expect(useToast().items.at(-1)?.message).toBe("Warte, bis das Cover gespeichert ist.");
    answer(shelfItem({ id: 30, personal_cover: true }));
    await vi.waitFor(() => expect(wrapper.emitted("close")).toHaveLength(1));
    wrapper.unmount();
  });

  it("applies a choice at once, moves the ring to it, and hands the entry to the page", async () => {
    coverCandidates.mockResolvedValue({
      partial: false,
      items: [candidate({ key: "media:cat", source: "catalog", chosen: true }), candidate({ key: "ol:2" })],
    });
    const saved = shelfItem({ id: 30, personal_cover: true, book: { id: 4, cover_public_id: "ol2" } });
    chooseCover.mockResolvedValue(saved);
    const wrapper = mountSheet();
    await flushPromises();
    await tiles(wrapper)[1]!.trigger("click");
    expect(chooseCover).toHaveBeenCalledWith(30, "ol:2");
    expect(tiles(wrapper)[1]!.classes()).toContain("pressing");
    // Escape while it saves keeps the sheet, so the answer still reaches the page.
    await wrapper.get(".sheet").trigger("keydown", { key: "Escape" });
    expect(wrapper.emitted("close")).toBeUndefined();
    await flushPromises();
    expect(tiles(wrapper).map((tile) => tile.attributes("aria-pressed"))).toEqual(["false", "true"]);
    await vi.waitFor(() => expect(wrapper.emitted("close")).toHaveLength(1));
    // Undo goes back to the standard cover the reader saw before.
    expect(wrapper.emitted("changed")).toEqual([[saved, { key: null }]]);
    wrapper.unmount();
  });

  it("closes without a request when the chosen cover is tapped again", async () => {
    coverCandidates.mockResolvedValue({ partial: false, items: [candidate({ key: "ol:2", chosen: true })] });
    const wrapper = mountSheet({ personalCover: true });
    await flushPromises();
    await tiles(wrapper)[0]!.trigger("click");
    expect(chooseCover).not.toHaveBeenCalled();
    expect(wrapper.emitted("close")).toHaveLength(1);
    wrapper.unmount();
  });

  it("keeps the sheet open and says so when a choice fails", async () => {
    coverCandidates.mockResolvedValue({ partial: false, items: [candidate({ key: "ol:2" })] });
    chooseCover.mockRejectedValue(new ApiError("Unknown cover", 400, "Unknown cover"));
    const wrapper = mountSheet();
    await flushPromises();
    await tiles(wrapper)[0]!.trigger("click");
    await flushPromises();
    expect(useToast().items.at(-1)?.message).toBe("Unknown cover");
    expect(wrapper.emitted("changed")).toBeUndefined();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(tiles(wrapper)[0]!.attributes("aria-pressed")).toBe("false");
    wrapper.unmount();
  });

  it("offers the standard cover back while the reader has their own, with the previous choice to undo to", async () => {
    coverCandidates.mockResolvedValue({ partial: false, items: [candidate({ key: "media:own", source: "photo", chosen: true })] });
    const standard = shelfItem({ id: 30, personal_cover: false });
    clearPersonalCover.mockResolvedValue(standard);
    const wrapper = mountSheet({ personalCover: true, coverPublicId: "own" });
    await flushPromises();
    expect(tiles(wrapper)[0]!.get(".tile-label").text()).toBe("Dein Foto");
    const row = wrapper.get(".standard-cover");
    expect(row.text()).toContain("Standard-Cover");
    // The catalog's cover (the entry's edition's, else the work's), not one "everyone" sees.
    expect(row.text()).toContain("Das Cover aus dem Katalog");
    await row.trigger("click");
    expect(clearPersonalCover).toHaveBeenCalledWith(30);
    await vi.waitFor(() => expect(wrapper.emitted("close")).toHaveLength(1));
    expect(wrapper.emitted("changed")).toEqual([[standard, { key: "media:own" }]]);
    wrapper.unmount();
  });

  it("falls back to the shown picture for Undo when no candidate is marked chosen", async () => {
    coverCandidates.mockRejectedValue(new TypeError("Failed to fetch"));
    const wrapper = mountSheet({ personalCover: true, coverPublicId: "abc" });
    await flushPromises();
    expect(wrapper.get(".error").text()).toBe("Die Cover konnten nicht geladen werden.");
    const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
    const input = wrapper.get(".own-photo input[type=file]");
    Object.defineProperty(input.element, "files", { value: [photo], configurable: true });
    await input.trigger("change");
    // The page uploads it once the sheet is gone; the sheet uploads nothing.
    expect(wrapper.emitted("picked")).toEqual([[photo, { key: "media:abc" }]]);
    expect(uploadPersonalCover).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});

describe("the page's half of a cover change", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const mock of [chooseCover, uploadPersonalCover, clearPersonalCover]) mock.mockReset();
    resetScan();
    i18n.global.locale.value = "de";
  });

  it("scans the own photo, then uploads what the reader kept", async () => {
    const flat = new File(["flat"], "flat.jpg", { type: "image/jpeg" });
    scanAnswer(() => flat);
    const saved = shelfItem({ id: 30, personal_cover: true });
    uploadPersonalCover.mockResolvedValue(saved);
    const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
    expect(await sendPersonalCover(30, photo)).toEqual(saved);
    expect(uploadPersonalCover).toHaveBeenCalledWith(30, flat);
  });

  it("uploads nothing when the reader cancels the scan, and says why an upload failed", async () => {
    scanAnswer(() => null);
    expect(await sendPersonalCover(30, new File(["x"], "a.jpg"))).toBeNull();
    expect(uploadPersonalCover).not.toHaveBeenCalled();
    resetScan();
    uploadPersonalCover.mockRejectedValue(new ApiError("Picture too large", 413, "Picture too large"));
    expect(await sendPersonalCover(30, new File(["x"], "a.jpg"))).toBeNull();
    expect(useToast().items.at(-1)?.message).toBe("Picture too large");
  });

  it("says Cover geändert with an Undo that restores the previous choice", async () => {
    const adopt = vi.fn();
    const back = shelfItem({ id: 30, personal_cover: true });
    chooseCover.mockResolvedValue(back);
    announceCoverChange(30, { key: "ol:2" }, adopt);
    const toast = useToast().items.at(-1)!;
    expect(toast.message).toBe("Cover geändert");
    expect(toast.action?.label).toBe("Rückgängig");
    await useToast().run(toast.id);
    expect(chooseCover).toHaveBeenCalledWith(30, "ol:2");
    expect(adopt).toHaveBeenCalledWith(back);

    const standard = shelfItem({ id: 30, personal_cover: false });
    clearPersonalCover.mockResolvedValue(standard);
    announceCoverChange(30, { key: null }, adopt);
    await useToast().run(useToast().items.at(-1)!.id);
    expect(clearPersonalCover).toHaveBeenCalledWith(30);
    expect(adopt).toHaveBeenLastCalledWith(standard);
  });

  it("offers no Undo when the previous cover is unknown, and says when an Undo fails", async () => {
    announceCoverChange(30, null, vi.fn());
    expect(useToast().items.at(-1)?.action).toBeNull();
    chooseCover.mockRejectedValue(new TypeError("Failed to fetch"));
    const adopt = vi.fn();
    announceCoverChange(30, { key: "ol:2" }, adopt);
    await useToast().run(useToast().items.at(-1)!.id);
    expect(adopt).not.toHaveBeenCalled();
    expect(useToast().items.at(-1)?.message).toBe("Dein voriges Cover konnte nicht zurückgeholt werden.");
  });
});

describe("the publisher on a tile", () => {
  it("drops the legal form at the end and nothing else", () => {
    expect(shortPublisher("Diogenes Verlag AG")).toBe("Diogenes Verlag");
    expect(shortPublisher("dtv Verlagsgesellschaft mbH & Co. KG")).toBe("dtv Verlagsgesellschaft");
    expect(shortPublisher("Penguin Random House LLC")).toBe("Penguin Random House");
    expect(shortPublisher("Vintage Books, Inc.")).toBe("Vintage Books");
    expect(shortPublisher("S. Fischer Verlag GmbH")).toBe("S. Fischer Verlag");
    expect(shortPublisher("Agora")).toBe("Agora");
    expect(shortPublisher("AG")).toBe("AG");
    expect(shortPublisher("  Suhrkamp ")).toBe("Suhrkamp");
  });
});
