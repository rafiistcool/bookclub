import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookDetail, bookOut } from "../test/fixtures";
import type { BookDetail } from "../types";

const uploadCover = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { uploadCover: (...args: unknown[]) => uploadCover(...args) },
}));

import { i18n } from "../i18n";
import { useToast } from "../stores/toast";
import CoverPrompt from "./CoverPrompt.vue";

/** A catalog book without a cover that the viewer has on their shelf, in their edition 13. */
function shelved(overrides: Partial<BookDetail> = {}): BookDetail {
  return bookDetail({ id: 9, shelf_id: 30, on_shelf: "currently_reading", edition_id: 13, source: "legacy", ...overrides });
}

describe("CoverPrompt", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    uploadCover.mockReset();
    i18n.global.locale.value = "en";
  });

  it("asks a reader with the book on their shelf for a photo of its cover", () => {
    const wrapper = mount(CoverPrompt, { props: { book: shelved() } });
    expect(wrapper.get("h2").text()).toBe("This book has no cover yet");
    expect(wrapper.text()).toContain("It's on your shelf.");
    expect(wrapper.text()).toContain("Photograph only the cover.");
    const action = wrapper.get("label.cover-photo");
    expect(action.text()).toBe("Add a photo of the cover");
    expect(action.classes()).toContain("btn-primary");
  });

  it("shows nothing off the shelf, with a cover, or for a book a reader added", () => {
    for (const book of [
      shelved({ shelf_id: null, on_shelf: null, edition_id: null }),
      shelved({ cover_url: "/media/abc/l", cover_thumb_url: "/media/abc/s" }),
      shelved({ custom: true, source: "user" }),
    ]) {
      const wrapper = mount(CoverPrompt, { props: { book } });
      expect(wrapper.find(".cover-prompt").exists()).toBe(false);
    }
  });

  it("uploads the photo for the reader's edition and hands the new cover on", async () => {
    const covered = bookOut({ id: 9, cover_url: "/media/new/l", cover_thumb_url: "/media/new/s", cover_public_id: "new" });
    uploadCover.mockResolvedValue({ book: covered });
    const wrapper = mount(CoverPrompt, { props: { book: shelved() } });
    const input = wrapper.get("input[type=file]");
    const photo = new File(["x"], "front.jpg", { type: "image/jpeg" });
    Object.defineProperty(input.element, "files", { value: [photo], configurable: true });
    await input.trigger("change");
    await flushPromises();
    expect(uploadCover).toHaveBeenCalledWith(9, photo, 13);
    expect(wrapper.emitted("covered")).toEqual([[covered]]);
    expect(useToast().items.at(-1)?.message).toBe("Your photo is the cover now");
  });

  it("speaks German", () => {
    i18n.global.locale.value = "de";
    const wrapper = mount(CoverPrompt, { props: { book: shelved() } });
    expect(wrapper.get("h2").text()).toBe("Dieses Buch hat noch kein Cover");
    expect(wrapper.get("label.cover-photo").text()).toBe("Foto vom Cover hinzufügen");
  });
});
