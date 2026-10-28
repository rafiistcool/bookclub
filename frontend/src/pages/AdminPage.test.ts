import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { EditRequestOut, ReportOut } from "../api/generated";
import { bookDetail, bookOut, edition, invite, user } from "../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  book: vi.fn(),
  edition: vi.fn(),
  adminEditWork: vi.fn(),
  adminEditEdition: vi.fn(),
  adminEditRequests: vi.fn(),
  approveEditRequest: vi.fn(),
  rejectEditRequest: vi.fn(),
  adminReports: vi.fn(),
  resolveReport: vi.fn(),
  adminInvites: vi.fn(),
  createSignupInvite: vi.fn(),
};

vi.mock("../api/client", () => {
  class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import AdminPage from "./AdminPage.vue";

const ADA = { id: 2, username: "ada", avatar_url: null };

function editRequest(overrides: Partial<EditRequestOut> = {}): EditRequestOut {
  return {
    id: 1,
    kind: "work_edit",
    status: "open",
    work_id: 9,
    edition_id: null,
    book: bookOut({ id: 9, title: "Circe" }),
    payload: {} as Record<string, never>,
    current: {} as Record<string, never>,
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    note: "",
    created_by: ADA,
    created_at: "2026-10-27T10:00:00Z",
    decided_by: null,
    decided_at: null,
    decision_note: "",
    ...overrides,
  };
}

function report(overrides: Partial<ReportOut> = {}): ReportOut {
  return {
    id: 7,
    reason: "wrong_book",
    details: "That is a cookbook.",
    status: "open",
    media_public_id: "abc",
    removed: false,
    cover_url: "/media/abc/l",
    cover_thumb_url: "/media/abc/s",
    book: bookOut({ id: 9, title: "Circe" }),
    created_by: ADA,
    created_at: "2026-10-27T10:00:00Z",
    resolved_by: null,
    resolved_at: null,
    ...overrides,
  };
}

async function mountAdmin(path = "/admin") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ username: "root", is_admin: true });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/admin", component: AdminPage },
      { path: "/:rest(.*)*", component: { template: "<p />" } },
    ],
  });
  await router.push(path);
  const wrapper = mount(AdminPage, { global: { plugins: [pinia, router, i18n], stubs: { BookCover: true } } });
  await flushPromises();
  return { wrapper, router };
}

function buttonIn(scope: ReturnType<Awaited<ReturnType<typeof mountAdmin>>["wrapper"]["get"]>, label: string) {
  return scope.findAll("button").find((node) => node.text() === label)!;
}

describe("AdminPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.book.mockRejectedValue(new ApiError("No such book.", 404, "No such book."));
    i18n.global.locale.value = "en";
  });

  it("lists the open catalog requests with what they propose, and approves one", async () => {
    api.adminEditRequests.mockResolvedValue({
      items: [
        editRequest({ payload: { title: "Circe", pages: 393, subtitle: null } as never, note: "From the back cover." }),
        editRequest({ id: 2, kind: "work_merge", payload: { into_work_id: 31 } as never }),
      ],
    });
    const { wrapper } = await mountAdmin();
    const items = wrapper.findAll(".request-item");
    expect(items).toHaveLength(2);
    expect(items[0]!.text()).toContain("Book details");
    expect(items[0]!.findAll("dt").map((node) => node.text())).toEqual(["Title", "Pages", "Subtitle"]);
    expect(items[0]!.findAll("dd").map((node) => node.text())).toEqual(["Circe", "393", "(clear it)"]);
    expect(items[0]!.get(".request-note").text()).toBe("From the back cover.");
    expect(items[1]!.text()).toContain("Same book");
    expect(items[1]!.findAll("a").at(-1)!.attributes("href")).toBe("/book/31");
    expect(wrapper.get(".admin-tabs button").text()).toBe("Requests 2");

    api.approveEditRequest.mockResolvedValue(editRequest({ status: "approved" }));
    await buttonIn(items[0]!, "Approve").trigger("click");
    await flushPromises();
    expect(api.approveEditRequest).toHaveBeenCalledWith(1);
    expect(wrapper.findAll(".request-item")).toHaveLength(1);
    expect(useToast().items.at(-1)?.message).toBe("Approved");
  });

  it("reads only the merge target, which the queue names without carrying it", async () => {
    api.adminEditRequests.mockResolvedValue({
      items: [
        editRequest({ id: 1, kind: "edition_edit", edition_id: 13, payload: { pages: 530 } as never }),
        editRequest({ id: 2, kind: "cover_replace", edition_id: 13, cover_thumb_url: "/media/p/s", cover_url: "/media/p/l" }),
        editRequest({ id: 3, kind: "work_merge", work_id: 40, book: bookOut({ id: 40, title: "Der Mörder" }), payload: { into_work_id: 31 } as never }),
        editRequest({ id: 4, kind: "work_edit", work_id: 77, book: null, payload: { title: "X" } as never }),
      ],
    });
    api.book.mockImplementation(async (id: number) => {
      if (id === 31) return bookDetail({ id: 31, title: "Ich bin Circe", authors: "Madeline Miller" });
      throw new ApiError("No such book.", 404, "No such book.");
    });
    const { wrapper } = await mountAdmin();
    const items = wrapper.findAll(".request-item");
    expect(items[0]!.get(".admin-book-link").text()).toContain("Circe");
    expect(items[1]!.get(".admin-book-link").text()).toContain("Circe");
    expect(items[1]!.get(".proposed-cover").text()).toContain("Proposed photo");
    expect(items[2]!.get(".merge-target").text()).toContain("Ich bin Circe");
    expect(items[2]!.get(".merge-target a").attributes("href")).toBe("/book/31");
    // A request without its book (the work is gone) says so; no read by its work id.
    expect(items[3]!.text()).toContain("This book is no longer in the catalog.");
    expect(api.book.mock.calls.map(([id]) => id)).toEqual([31]);
  });

  it("rejects with a note, and drops a request someone else decided", async () => {
    api.adminEditRequests.mockResolvedValue({ items: [editRequest(), editRequest({ id: 2 })] });
    const { wrapper } = await mountAdmin();
    const first = wrapper.findAll(".request-item")[0]!;
    await buttonIn(first, "Reject").trigger("click");
    await first.get("textarea[name=reject-note]").setValue("  Not what the book says. ");
    api.rejectEditRequest.mockResolvedValue(editRequest({ status: "rejected" }));
    await first.get("form").trigger("submit");
    await flushPromises();
    expect(api.rejectEditRequest).toHaveBeenCalledWith(1, "Not what the book says.");
    expect(wrapper.findAll(".request-item")).toHaveLength(1);

    api.approveEditRequest.mockRejectedValue(new ApiError("Someone already decided this request", 409, "This request was already decided"));
    await buttonIn(wrapper.get(".request-item"), "Approve").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".request-item")).toHaveLength(0);
    expect(wrapper.text()).toContain("Nothing waiting.");
    expect(useToast().items.at(-1)?.message).toBe("Someone already decided this request");
  });

  it("takes a reported cover down with a second tap, or keeps it", async () => {
    api.adminReports.mockResolvedValue({
      items: [report(), report({ id: 8, reason: "copyright", details: "", media_public_id: "def" })],
    });
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(api.adminEditRequests).not.toHaveBeenCalled();
    const items = wrapper.findAll(".report-item");
    expect(items[0]!.text()).toContain("It shows a different book");
    expect(items[0]!.text()).toContain("That is a cookbook.");

    api.resolveReport.mockResolvedValue(report({ status: "removed", removed: true }));
    await buttonIn(items[0]!, "Take down").trigger("click");
    expect(api.resolveReport).not.toHaveBeenCalled();
    await buttonIn(items[0]!, "Take down for good").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenCalledWith(7, "remove");
    expect(useToast().items.at(-1)?.message).toBe("Picture taken down");

    api.resolveReport.mockResolvedValue(report({ id: 8, status: "dismissed" }));
    await buttonIn(wrapper.get(".report-item"), "Keep it").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenLastCalledWith(8, "dismiss");
    expect(wrapper.text()).toContain("No open reports.");
  });

  it("shows the record's value now beside each proposed one", async () => {
    api.adminEditRequests.mockResolvedValue({
      items: [
        editRequest({
          payload: { title: "Circe", pages: 393, subtitle: null, year: 2018 } as never,
          current: { title: "Cirse", pages: null, subtitle: "A Novel", year: 2018 } as never,
        }),
      ],
    });
    const { wrapper } = await mountAdmin();
    const item = wrapper.get(".request-item");
    expect(item.findAll("dt").map((node) => node.text())).toEqual(["Title", "Pages", "Subtitle", "First published"]);
    expect(item.findAll(".field-before .field-value").map((node) => node.text())).toEqual(["Cirse", "(empty)", "A Novel", "2018"]);
    expect(item.findAll(".field-before.was-empty")).toHaveLength(1);
    expect(item.findAll(".field-after .field-value").map((node) => node.text())).toEqual(["Circe", "393", "(clear it)", "2018"]);
    expect(item.findAll(".field-after")[3]!.text()).toContain("already so");
    expect(item.findAll(".field-before .field-tag")[0]!.text()).toBe("Now");
    expect(item.findAll(".field-after .field-tag")[0]!.text()).toBe("Proposed");

    i18n.global.locale.value = "de";
    await flushPromises();
    expect(item.findAll(".field-before .field-value")[1]!.text()).toBe("(leer)");
    expect(item.findAll(".field-before .field-tag")[0]!.text()).toBe("Jetzt");
  });

  it("edits a request's book as admin, and reads the queue again", async () => {
    api.adminEditRequests.mockResolvedValue({
      items: [editRequest({ payload: { title: "Circe" } as never, current: { title: "Cirse" } as never })],
    });
    api.book.mockResolvedValue(bookDetail({ id: 9, title: "Cirse", subtitle: "", authors: "Madeline Miller" }));
    const { wrapper } = await mountAdmin();
    await buttonIn(wrapper.get(".request-item"), "Edit as admin").trigger("click");
    await flushPromises();
    expect(api.book).toHaveBeenCalledWith(9);
    const sheet = wrapper.get(".sheet");
    expect(sheet.text()).toContain("Edit this book as admin");
    expect(sheet.find("textarea[name=note]").exists()).toBe(false);
    await sheet.get("input[name=title]").setValue("Circe (corrected)");
    api.adminEditWork.mockResolvedValue(bookDetail({ id: 9, title: "Circe (corrected)" }));
    await sheet.get("form").trigger("submit");
    await flushPromises();
    expect(api.adminEditWork).toHaveBeenCalledWith(9, { title: "Circe (corrected)" });
    expect(api.adminEditRequests).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".sheet").exists()).toBe(false);
    expect(useToast().items.at(-1)?.message).toBe("Saved. Everyone sees the new values now.");
    // The request stays open for a decision.
    expect(api.approveEditRequest).not.toHaveBeenCalled();
    expect(api.rejectEditRequest).not.toHaveBeenCalled();
  });

  it("edits the edition a request names, with the server's 400 in the sheet", async () => {
    api.adminEditRequests.mockResolvedValue({
      items: [editRequest({ kind: "edition_edit", edition_id: 13, payload: { pages: 530 } as never })],
    });
    api.edition.mockResolvedValue(edition({ id: 13, work_id: 9, pages: 500, publisher: "Klett" }));
    const { wrapper } = await mountAdmin();
    await buttonIn(wrapper.get(".request-item"), "Edit as admin").trigger("click");
    await flushPromises();
    expect(api.edition).toHaveBeenCalledWith(13);
    expect(api.book).not.toHaveBeenCalled();
    const sheet = wrapper.get(".sheet");
    expect(sheet.text()).toContain("Edit this edition as admin");
    await sheet.get("input[name=pages]").setValue(530);
    api.adminEditEdition.mockRejectedValue(new ApiError("Invalid value for pages", 400, "Invalid value for pages"));
    await sheet.get("form").trigger("submit");
    await flushPromises();
    expect(api.adminEditEdition).toHaveBeenCalledWith(13, { pages: 530 });
    expect(wrapper.get(".sheet").text()).toContain("Invalid value for pages");
    expect(api.adminEditRequests).toHaveBeenCalledTimes(1);
  });

  it("closes every open report of a picture with one decision", async () => {
    api.adminReports.mockResolvedValue({
      items: [
        report(),
        report({ id: 9, created_by: { id: 3, username: "grace", avatar_url: null } }),
        report({ id: 8, media_public_id: "def" }),
      ],
    });
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(wrapper.findAll(".report-item")).toHaveLength(3);
    api.resolveReport.mockResolvedValue(report({ status: "dismissed" }));
    await buttonIn(wrapper.findAll(".report-item")[1]!, "Keep it").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenCalledWith(9, "dismiss");
    expect(wrapper.findAll(".report-item")).toHaveLength(1);
    expect(wrapper.get(".report-item").text()).toContain("That is a cookbook.");
    expect(wrapper.findAll(".admin-tabs button")[1]!.text()).toBe("Reports 1");
  });

  it("creates a sign-up invite with the limits given, and lists the others", async () => {
    api.adminInvites.mockResolvedValue({
      items: [invite({ id: 1, max_uses: 5, use_count: 2, used_by: ["ada", "grace"], created_by: "root" })],
    });
    const { wrapper, router } = await mountAdmin();
    await wrapper.findAll(".admin-tabs button").find((node) => node.text().startsWith("Invites"))!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.query).toEqual({ tab: "invites" });
    expect(wrapper.get(".invite-row").text()).toContain("For 5 people");
    expect(wrapper.get(".invite-row").text()).toContain("Used by ada, grace");

    const fresh = invite({ id: 2, code: "AAAA2222BB", max_uses: null, expires_at: "2026-11-11T00:00:00Z" });
    api.createSignupInvite.mockResolvedValue(fresh);
    await wrapper.get("input[name=days]").setValue("14");
    await wrapper.get("form.invite-form").trigger("submit");
    await flushPromises();
    expect(api.createSignupInvite).toHaveBeenCalledWith({ expires_in_days: 14 });
    expect(wrapper.get(".fresh-invite .invite-url").text()).toBe(fresh.url);
    expect(wrapper.findAll(".invite-row")).toHaveLength(2);
    expect(wrapper.findAll(".invite-row")[0]!.text()).toContain("Any number of people");

    await wrapper.get("form.invite-form").trigger("submit");
    await flushPromises();
    expect(api.createSignupInvite).toHaveBeenLastCalledWith({});
  });
});
