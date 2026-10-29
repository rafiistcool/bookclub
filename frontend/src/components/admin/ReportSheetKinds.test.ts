// The report sheet for every kind of #54 (Q11 option B: covers, profile pictures, and the books
// and editions readers added) and the "Report" actions that open it. It tests
// components/ReportSheet.vue, pages/MemberShelfPage.vue, pages/BookDetailPage.vue and
// components/EditRequestSheet.vue, which ADM2 owns, but lives here because their colocated test
// files are outside ADM2's files (ADM's precedent: BookDetailAdminEdit.test.ts,
// ReportCoverSheetRemoved.test.ts; docs/phase2/notes/adm2.md).
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookDetail, edition, user } from "../../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  reportCover: vi.fn(),
  reportWork: vi.fn(),
  reportEdition: vi.fn(),
  friendShelf: vi.fn(),
  book: vi.fn(),
  editions: vi.fn(),
  diary: vi.fn(),
};

vi.mock("../../api/client", () => {
  class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../../api/client";
import { i18n } from "../../i18n";
import BookDetailPage from "../../pages/BookDetailPage.vue";
import MemberShelfPage from "../../pages/MemberShelfPage.vue";
import { useService } from "../../stores/service";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import EditRequestSheet from "../EditRequestSheet.vue";
import ReportSheet, { picturePublicId, REMOVED_TITLE, type ReportTarget } from "../ReportSheet.vue";

const PICTURE = "abcdefghijklmnopqrstuvwxyz";
const COVER: ReportTarget = { kind: "cover", publicId: "abc2def", title: "Circe" };

function mountSheet(target: ReportTarget) {
  return mount(ReportSheet, { props: { target } });
}

function reasonsOf(wrapper: ReturnType<typeof mountSheet>) {
  return wrapper.findAll('input[name="reason"]').map((node) => node.attributes("value"));
}

describe("ReportSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
  });

  it("sends the reason and details for the cover's media id", async () => {
    api.reportCover.mockResolvedValue({ id: 1, status: "open" });
    const wrapper = mountSheet(COVER);
    expect(wrapper.text()).toContain("Report this cover");
    expect(reasonsOf(wrapper)).toEqual(["wrong_book", "offensive", "copyright", "other"]);
    const send = wrapper.get('button[type="submit"]');
    expect(send.attributes("disabled")).toBeDefined();
    await wrapper.get('input[value="wrong_book"]').setValue(true);
    await wrapper.get("textarea").setValue("  It shows Galatea.  ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportCover).toHaveBeenCalledWith("abc2def", { reason: "wrong_book", details: "It shows Galatea." });
    expect(useToast().items[0]?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("keeps the sheet open with the server's answer when the report fails", async () => {
    api.reportCover.mockRejectedValue(new ApiError("Slow down a little. Try again later.", 429, "Slow down a little. Try again later."));
    const wrapper = mountSheet(COVER);
    await wrapper.get('input[value="other"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportCover.mock.calls[0]![1]).toEqual({ reason: "other", details: null });
    expect(wrapper.text()).toContain("Slow down a little. Try again later.");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("says so when the picture was already taken down", async () => {
    api.reportCover.mockResolvedValue({ id: 4, status: "removed" });
    const wrapper = mountSheet(COVER);
    await wrapper.get('input[value="offensive"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportCover).toHaveBeenCalledWith("abc2def", { reason: "offensive", details: null });
    expect(useToast().items[0]?.message).toBe("This picture was already taken down. Thanks anyway.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("reports a profile picture by its public id, without the wrong-book reason", async () => {
    api.reportCover.mockResolvedValue({ id: 5, status: "open" });
    const wrapper = mountSheet({ kind: "avatar", publicId: PICTURE, username: "grace" });
    expect(wrapper.text()).toContain("Report this profile picture");
    expect(wrapper.text()).toContain("Profile picture of grace");
    expect(reasonsOf(wrapper)).toEqual(["offensive", "copyright", "other"]);
    await wrapper.get('input[value="copyright"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportCover).toHaveBeenCalledWith(PICTURE, { reason: "copyright", details: null });
    expect(api.reportWork).not.toHaveBeenCalled();
  });

  it("reports a book or an edition a reader added through their own routes", async () => {
    api.reportWork.mockResolvedValue({ id: 6, status: "open" });
    const work = mountSheet({ kind: "work", workId: 4, title: "Manual Zine" });
    expect(work.text()).toContain("Report this book");
    expect(work.text()).toContain("Manual Zine");
    expect(reasonsOf(work)).toEqual(["offensive", "copyright", "other"]);
    await work.get('input[value="offensive"]').setValue(true);
    await work.get("textarea").setValue("The description insults a reader.");
    await work.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportWork).toHaveBeenCalledWith(4, { reason: "offensive", details: "The description insults a reader." });

    api.reportEdition.mockResolvedValue({ id: 7, status: "open" });
    i18n.global.locale.value = "de";
    const edition = mountSheet({ kind: "edition", editionId: 13, title: "Manual Zine" });
    expect(edition.text()).toContain("Diese Ausgabe melden");
    await edition.get('input[value="other"]').setValue(true);
    await edition.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportEdition).toHaveBeenCalledWith(13, { reason: "other", details: null });
    expect(useToast().items.at(-1)?.message).toBe("Danke. Jemand aus dem Admin-Team sieht es sich an.");
    expect(api.reportCover).not.toHaveBeenCalled();
  });

  it("names the contact address for people without an account, when the server has one", async () => {
    expect(mountSheet(COVER).find(".report-contact").exists()).toBe(false);
    useService().$patch({ contact_email: "abuse@bookclub.test" });
    const wrapper = mountSheet(COVER);
    const line = wrapper.get(".report-contact");
    expect(line.text()).toBe("No account here? Anyone can report content by writing to abuse@bookclub.test.");
    expect(line.get("a").attributes("href")).toBe("mailto:abuse@bookclub.test");
  });

  it("reads the public id from a picture's address", () => {
    expect(picturePublicId(`/media/${PICTURE}/a`)).toBe(PICTURE);
    expect(picturePublicId(`/media/${PICTURE}/l`)).toBe(PICTURE);
    expect(picturePublicId(null)).toBeNull();
    expect(picturePublicId("https://elsewhere.example/media/x/a")).toBeNull();
    expect(picturePublicId(`/media/${PICTURE.toUpperCase()}/a`)).toBeNull();
  });
});

// The "Report" actions of #54 on pages ADM2 owns (pages/MemberShelfPage.vue,
// pages/BookDetailPage.vue and components/EditRequestSheet.vue).
describe("the report actions", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
  });

  async function memberPage(name: string, avatarUrl: string | null, viewer = "ada") {
    api.friendShelf.mockResolvedValue({
      timezone: "UTC",
      user: { id: 2, username: name, avatar_url: avatarUrl },
      items: [],
      favorites: [],
    });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/u/:username", component: MemberShelfPage }] });
    await router.push(`/u/${name}`);
    await router.isReady();
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ username: viewer });
    const wrapper = mount(MemberShelfPage, { global: { plugins: [pinia, router] } });
    await flushPromises();
    return wrapper;
  }

  it("offers to report another member's profile picture, and sends its public id", async () => {
    const wrapper = await memberPage("grace", `/media/${PICTURE}/a`);
    await wrapper.get("button.report-avatar").trigger("click");
    expect(wrapper.text()).toContain("Report this profile picture");
    expect(wrapper.text()).toContain("Profile picture of grace");
    api.reportCover.mockResolvedValue({ id: 9, status: "open" });
    await wrapper.get('input[value="offensive"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportCover).toHaveBeenCalledWith(PICTURE, { reason: "offensive", details: null });
    expect(wrapper.find("form").exists()).toBe(false);
  });

  it("offers no report without a picture, or on the reader's own page", async () => {
    expect((await memberPage("grace", null)).find("button.report-avatar").exists()).toBe(false);
    expect((await memberPage("ada", `/media/${PICTURE}/a`)).find("button.report-avatar").exists()).toBe(false);
  });

  async function bookPage(custom: boolean, title = "Manual Zine") {
    api.book.mockResolvedValue(
      bookDetail({
        id: 4,
        ol_work_key: custom ? null : "/works/OL4W",
        title,
        source: custom ? "user" : "openlibrary",
        custom,
      }),
    );
    api.editions.mockResolvedValue({ items: [] });
    api.diary.mockResolvedValue({ work_id: 4, my_progress: null, my_status: null, items: [], timezone: "UTC" });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/book/:id", component: BookDetailPage }] });
    await router.push("/book/4");
    await router.isReady();
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ clubs: [] });
    const wrapper = mount(BookDetailPage, { global: { plugins: [pinia, router] } });
    await flushPromises();
    return wrapper;
  }

  it("offers to report a book only when a reader added it", async () => {
    expect((await bookPage(false)).find("button.report-book").exists()).toBe(false);

    const wrapper = await bookPage(true);
    await wrapper.get("button.report-book").trigger("click");
    expect(wrapper.get(".sheet").text()).toContain("Report this book");
    api.reportWork.mockResolvedValue({ id: 10, status: "open" });
    await wrapper.get('.sheet input[value="copyright"]').setValue(true);
    await wrapper.get(".sheet form").trigger("submit");
    await flushPromises();
    expect(api.reportWork).toHaveBeenCalledWith(4, { reason: "copyright", details: null });
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at it.");
  });

  it("shows the placeholder title of a book whose text an admin removed in the reader's language", async () => {
    i18n.global.locale.value = "de";
    const removed = await bookPage(true, REMOVED_TITLE);
    expect(removed.get("h1").text()).toBe("[entfernt]");
    expect(removed.text()).toContain("Ein Admin hat nach einer Meldung entfernt, was jemand für dieses Buch eingetragen hatte.");

    i18n.global.locale.value = "en";
    // Only a book a reader added: a catalog book with that title is shown as it is.
    const catalog = await bookPage(false, REMOVED_TITLE);
    expect(catalog.get("h1").text()).toBe("[removed]");
    expect(catalog.text()).not.toContain("after a report");
  });

  function correctionSheet(props: InstanceType<typeof EditRequestSheet>["$props"]) {
    const pinia = createPinia();
    setActivePinia(pinia);
    return mount(EditRequestSheet, { props, global: { plugins: [pinia] } });
  }

  it("reports an edition a reader added from its correction sheet", async () => {
    const own = edition({ id: 13, title: "Mein Heft", publisher: "Basement Press", source: "user" });
    const wrapper = correctionSheet({ edition: own });
    expect(wrapper.text()).toContain("Not a mistake, but offensive or infringing someone’s rights?");
    await wrapper.get(".report-instead button").trigger("click");
    expect(wrapper.text()).toContain("Report this edition");
    expect(wrapper.find('textarea[name="note"]').exists()).toBe(false);
    api.reportEdition.mockResolvedValue({ id: 11, status: "open" });
    await wrapper.get('input[value="offensive"]').setValue(true);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.reportEdition).toHaveBeenCalledWith(13, { reason: "offensive", details: null });
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("offers no report on a catalog edition's correction sheet, or in an admin's edit", () => {
    expect(correctionSheet({ edition: edition({ source: "dnb" }) }).find(".report-instead").exists()).toBe(false);
    expect(correctionSheet({ edition: edition({ source: "user" }), admin: true }).find(".report-instead").exists()).toBe(false);
    const book = correctionSheet({ book: bookDetail({ id: 4, title: "Manual Zine", source: "user", custom: true }) });
    expect(book.get(".report-instead button").text()).toBe("Report this book");
  });
});
