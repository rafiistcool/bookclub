import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut } from "../test/fixtures";

// No scanner is mounted here: the cover scan hands each photo back as it is.
vi.mock("../stores/coverScan", () => import("../test/coverScan"));

const uploadCover = vi.fn();
const adminSetCover = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    uploadCover: (...args: unknown[]) => uploadCover(...args),
    adminSetCover: (...args: unknown[]) => adminSetCover(...args),
  },
}));

import { useToast } from "../stores/toast";
import { resetScan, scanAnswer, scanned } from "../test/coverScan";
import CoverPhotoButton, { sendCoverPhoto } from "./CoverPhotoButton.vue";

async function pick(wrapper: ReturnType<typeof mount>, file: File) {
  const input = wrapper.get("input[type=file]");
  Object.defineProperty(input.element, "files", { value: [file], configurable: true });
  await input.trigger("change");
  await flushPromises();
}

describe("CoverPhotoButton", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    uploadCover.mockReset();
    adminSetCover.mockReset();
    resetScan();
  });

  it("uploads what the cover scan kept, and nothing when it was cancelled", async () => {
    const book = bookOut({ id: 9, cover_url: "/media/flat/c", cover_thumb_url: "/media/flat/t", cover_public_id: "flat" });
    uploadCover.mockResolvedValue({ book });
    const photo = new File(["photo"], "copy.jpg", { type: "image/jpeg" });
    const flat = new File(["flat"], "cover.jpg", { type: "image/jpeg" });
    scanAnswer(() => flat);
    const wrapper = mount(CoverPhotoButton, { props: { workId: 9, editionId: 3, hasCover: false } });
    await pick(wrapper, photo);
    expect(scanned).toEqual([photo]);
    expect(uploadCover).toHaveBeenCalledWith(9, flat, 3);
    expect(wrapper.emitted("covered")).toEqual([[book]]);

    uploadCover.mockClear();
    const toasts = useToast().items.length;
    scanAnswer(() => null);
    await pick(wrapper, photo);
    expect(uploadCover).not.toHaveBeenCalled();
    expect(useToast().items).toHaveLength(toasts);
    expect(wrapper.emitted("covered")).toHaveLength(1);
    expect(wrapper.text()).toBe("Add a photo of your copy");
  });

  it("checks the size of what the scan kept", async () => {
    const huge = new File(["x"], "cover.jpg", { type: "image/jpeg" });
    Object.defineProperty(huge, "size", { value: 51 * 1024 * 1024 });
    scanAnswer(() => huge);
    const wrapper = mount(CoverPhotoButton, { props: { workId: 9, editionId: null, hasCover: false } });
    await pick(wrapper, new File(["x"], "copy.jpg", { type: "image/jpeg" }));
    expect(uploadCover).not.toHaveBeenCalled();
    expect(useToast().items.at(-1)?.message).toContain("50 MB");
  });

  it("sets the first cover at once, for the reader's edition", async () => {
    const book = bookOut({ id: 9, cover_url: "/media/abc/c", cover_thumb_url: "/media/abc/t", cover_public_id: "abc" });
    uploadCover.mockResolvedValue({ book });
    const wrapper = mount(CoverPhotoButton, { props: { workId: 9, editionId: 3, hasCover: false } });
    expect(wrapper.text()).toBe("Add a photo of your copy");
    const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
    await pick(wrapper, photo);
    expect(uploadCover).toHaveBeenCalledWith(9, photo, 3);
    expect(wrapper.emitted("covered")).toEqual([[book]]);
    expect(useToast().items.at(-1)?.message).toBe("Your photo is the cover now");
  });

  it("says a photo waits for an admin when a cover exists", async () => {
    uploadCover.mockResolvedValue({ request_id: 12 });
    const wrapper = mount(CoverPhotoButton, { props: { workId: 9, editionId: null, hasCover: true } });
    expect(wrapper.text()).toBe("Suggest a photo of your copy");
    await pick(wrapper, new File(["x"], "copy.png", { type: "image/png" }));
    expect(wrapper.emitted("covered")).toBeUndefined();
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at your photo.");
  });

  it("refuses a file over 50 MB without uploading it", async () => {
    const wrapper = mount(CoverPhotoButton, { props: { workId: 9, editionId: null, hasCover: false } });
    const huge = new File(["x"], "huge.jpg", { type: "image/jpeg" });
    Object.defineProperty(huge, "size", { value: 51 * 1024 * 1024 });
    await pick(wrapper, huge);
    expect(uploadCover).not.toHaveBeenCalled();
    expect(useToast().items.at(-1)?.message).toContain("50 MB");
  });

  it("sets an instance admin's picture as the cover at once, with its own label and look", async () => {
    const book = bookOut({ id: 9, cover_url: "/media/adm/l", cover_thumb_url: "/media/adm/s", cover_public_id: "adm" });
    adminSetCover.mockResolvedValue({ book });
    const wrapper = mount(CoverPhotoButton, {
      props: { workId: 9, editionId: 13, hasCover: true, admin: true, label: "Upload a picture", look: "ghost" },
    });
    expect(wrapper.text()).toBe("Upload a picture");
    expect(wrapper.classes()).toEqual(expect.arrayContaining(["btn", "btn-ghost", "btn-sm"]));
    expect(wrapper.classes()).not.toContain("text-btn");
    const picture = new File(["x"], "cover.webp", { type: "image/webp" });
    await pick(wrapper, picture);
    expect(adminSetCover).toHaveBeenCalledWith(9, picture, 13);
    expect(uploadCover).not.toHaveBeenCalled();
    expect(wrapper.emitted("covered")).toEqual([[book]]);
    expect(useToast().items.at(-1)?.message).toBe("The cover is set for everyone");
  });

  it("only hands the chosen file on with pickOnly, so a closing sheet loses nothing", async () => {
    const wrapper = mount(CoverPhotoButton, {
      props: { workId: 9, editionId: null, hasCover: true, admin: true, pickOnly: true, look: "row", label: "Set cover as admin" },
    });
    expect(wrapper.element.tagName).toBe("LABEL");
    expect(wrapper.classes()).toEqual(expect.arrayContaining(["btn", "btn-ghost", "btn-block"]));
    const picture = new File(["x"], "cover.png", { type: "image/png" });
    await pick(wrapper, picture);
    expect(wrapper.emitted("picked")).toEqual([[picture]]);
    expect(wrapper.emitted("covered")).toBeUndefined();
    expect(adminSetCover).not.toHaveBeenCalled();
    expect(uploadCover).not.toHaveBeenCalled();
    // Never stuck on "Uploading…": the page uploads, not the row.
    expect(wrapper.text()).toBe("Set cover as admin");
  });

  it("uploads for a page with sendCoverPhoto and answers with the new cover, or null", async () => {
    const book = bookOut({ id: 9, cover_url: "/media/abc/c", cover_thumb_url: "/media/abc/t", cover_public_id: "abc" });
    const photo = new File(["x"], "copy.jpg", { type: "image/jpeg" });
    uploadCover.mockResolvedValueOnce({ book });
    expect(await sendCoverPhoto({ workId: 9, editionId: 3 }, photo)).toEqual(book);
    expect(uploadCover).toHaveBeenLastCalledWith(9, photo, 3);
    uploadCover.mockResolvedValueOnce({ request_id: 12 });
    expect(await sendCoverPhoto({ workId: 9, editionId: null }, photo)).toBeNull();
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at your photo.");
    uploadCover.mockRejectedValueOnce(new TypeError("Failed to fetch"));
    expect(await sendCoverPhoto({ workId: 9, editionId: null }, photo)).toBeNull();
    expect(useToast().items.at(-1)?.message).toBe("Could not upload the photo");
    adminSetCover.mockResolvedValueOnce({ book });
    expect(await sendCoverPhoto({ workId: 9, editionId: 13, admin: true }, photo)).toEqual(book);
    expect(adminSetCover).toHaveBeenLastCalledWith(9, photo, 13);
  });
});
