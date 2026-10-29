import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { AdminSummary, AdminUserOut, EditRequestOut, ReportOut } from "../api/generated";
import { bookDetail, bookOut, edition, invite, user } from "../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  book: vi.fn(),
  edition: vi.fn(),
  adminEditWork: vi.fn(),
  adminEditEdition: vi.fn(),
  adminSummary: vi.fn(),
  adminEditRequestPage: vi.fn(),
  approveEditRequest: vi.fn(),
  rejectEditRequest: vi.fn(),
  adminReportPage: vi.fn(),
  resolveReport: vi.fn(),
  adminTakeDown: vi.fn(),
  adminInvites: vi.fn(),
  createSignupInvite: vi.fn(),
  adminSuspendedUsers: vi.fn(),
  adminSuspendUser: vi.fn(),
  adminUnsuspendUser: vi.fn(),
};

vi.mock("../api/client", () => {
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

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import AdminPage from "./AdminPage.vue";

const ADA = { id: 2, username: "ada", avatar_url: null };
const GRACE = { id: 3, username: "grace", avatar_url: "/media/gracegracegracegracegrace/a" };

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
    target_kind: "cover",
    reason: "wrong_book",
    details: "That is a cookbook.",
    status: "open",
    via: "app",
    media_public_id: "abc",
    removed: false,
    cover_url: "/media/abc/l",
    cover_thumb_url: "/media/abc/s",
    avatar_url: null,
    target_user: null,
    book: bookOut({ id: 9, title: "Circe" }),
    created_by: ADA,
    created_at: "2026-10-27T10:00:00Z",
    resolved_by: null,
    resolved_at: null,
    closed_on_arrival: false,
    gone: false,
    ...overrides,
  };
}

/** A page of a queue, as `?status=&page=` answers. */
function page<T>(items: T[], hasMore = false, number = 1) {
  return { items, page: number, has_more: hasMore };
}

function queueSummary(open: number, overdue = 0) {
  return { open, overdue, oldest: open ? { id: 1, created_at: "2026-10-27T10:00:00Z" } : null };
}

function summary(overrides: Partial<AdminSummary> = {}): AdminSummary {
  return {
    target_hours: 72,
    reports: queueSummary(0),
    edit_requests: queueSummary(0),
    report_stats: { days: 90, resolved: 0, median_hours: null, max_hours: null, over_target: 0 },
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
    api.adminSummary.mockResolvedValue(summary());
    api.adminEditRequestPage.mockResolvedValue(page([]));
    api.adminReportPage.mockResolvedValue(page([]));
    i18n.global.locale.value = "en";
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("lists the open catalog requests with what they propose, and approves one", async () => {
    api.adminEditRequestPage.mockResolvedValue(
      page([
        editRequest({ payload: { title: "Circe", pages: 393, subtitle: null } as never, note: "From the back cover." }),
        editRequest({ id: 2, kind: "work_merge", payload: { into_work_id: 31 } as never }),
      ]),
    );
    api.adminSummary.mockResolvedValueOnce(summary({ edit_requests: queueSummary(2) }));
    api.adminSummary.mockResolvedValueOnce(summary({ edit_requests: queueSummary(1) }));
    const { wrapper } = await mountAdmin();
    expect(api.adminEditRequestPage).toHaveBeenCalledWith("open", 1);
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
    // The badges come from the summary, read again after the decision.
    expect(api.adminSummary).toHaveBeenCalledTimes(2);
    expect(wrapper.get(".admin-tabs button").text()).toBe("Requests 1");
  });

  it("reads only the merge target, which the queue names without carrying it", async () => {
    api.adminEditRequestPage.mockResolvedValue({
      page: 1,
      has_more: false,
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
    api.adminEditRequestPage.mockResolvedValue(page([editRequest(), editRequest({ id: 2 })]));
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
    api.adminReportPage.mockResolvedValue(
      page([report(), report({ id: 8, reason: "copyright", details: "", media_public_id: "def" })]),
    );
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(api.adminEditRequestPage).not.toHaveBeenCalled();
    expect(api.adminReportPage).toHaveBeenCalledWith("open", 1);
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
    api.adminEditRequestPage.mockResolvedValue(
      page([
        editRequest({
          payload: { title: "Circe", pages: 393, subtitle: null, year: 2018 } as never,
          current: { title: "Cirse", pages: null, subtitle: "A Novel", year: 2018 } as never,
        }),
      ]),
    );
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
    api.adminEditRequestPage.mockResolvedValue(
      page([editRequest({ payload: { title: "Circe" } as never, current: { title: "Cirse" } as never })]),
    );
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
    expect(api.adminEditRequestPage).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".sheet").exists()).toBe(false);
    expect(useToast().items.at(-1)?.message).toBe("Saved. Everyone sees the new values now.");
    // The request stays open for a decision.
    expect(api.approveEditRequest).not.toHaveBeenCalled();
    expect(api.rejectEditRequest).not.toHaveBeenCalled();
  });

  it("edits the edition a request names, with the server's 400 in the sheet", async () => {
    api.adminEditRequestPage.mockResolvedValue(
      page([editRequest({ kind: "edition_edit", edition_id: 13, payload: { pages: 530 } as never })]),
    );
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
    expect(api.adminEditRequestPage).toHaveBeenCalledTimes(1);
  });

  it("closes every open report of a picture with one decision", async () => {
    api.adminReportPage.mockResolvedValue(
      page([
        report(),
        report({ id: 9, created_by: { id: 3, username: "grace", avatar_url: null } }),
        report({ id: 8, media_public_id: "def" }),
      ]),
    );
    api.adminSummary.mockResolvedValueOnce(summary({ reports: queueSummary(3) }));
    api.adminSummary.mockResolvedValueOnce(summary({ reports: queueSummary(1) }));
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(wrapper.findAll(".admin-tabs button")[1]!.text()).toBe("Reports 3");
    expect(wrapper.findAll(".report-item")).toHaveLength(3);
    api.resolveReport.mockResolvedValue(report({ status: "dismissed" }));
    await buttonIn(wrapper.findAll(".report-item")[1]!, "Keep it").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenCalledWith(9, "dismiss");
    expect(wrapper.findAll(".report-item")).toHaveLength(1);
    expect(wrapper.get(".report-item").text()).toContain("That is a cookbook.");
    expect(wrapper.findAll(".admin-tabs button")[1]!.text()).toBe("Reports 1");
  });

  it("shows both badges on first load, with the overdue ones marked", async () => {
    api.adminSummary.mockResolvedValue(summary({ edit_requests: queueSummary(2), reports: queueSummary(3, 1) }));
    const { wrapper } = await mountAdmin();
    const tabs = wrapper.findAll(".admin-tabs button");
    expect(tabs.map((node) => node.text())).toEqual(["Requests 2", "Reports 3", "Invites", "Accounts"]);
    expect(tabs[0]!.get(".seg-count").classes()).not.toContain("overdue");
    expect(tabs[1]!.get(".seg-count").classes()).toContain("overdue");
    expect(tabs[1]!.get(".seg-count").attributes("title")).toBe("1 overdue");
    // Only the shown queue is read; the other badge comes from the summary.
    expect(api.adminReportPage).not.toHaveBeenCalled();
  });

  it("shows how long each open item has waited, and flags it after 72 hours", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-28T12:00:00Z"));
    api.adminEditRequestPage.mockResolvedValue(
      page([
        editRequest({ id: 1, created_at: "2026-10-28T11:30:00Z" }),
        editRequest({ id: 2, created_at: "2026-10-28T07:00:00Z" }),
        editRequest({ id: 3, created_at: "2026-10-24T12:00:00Z" }),
      ]),
    );
    const { wrapper } = await mountAdmin();
    const items = wrapper.findAll(".request-item");
    expect(items.map((item) => item.get(".age").text())).toEqual([
      "waiting less than an hour",
      "waiting 5 hours",
      "waiting 4 days",
    ]);
    expect(items.map((item) => item.find(".overdue-badge").exists())).toEqual([false, false, true]);
    expect(items[2]!.classes()).toContain("overdue");

    api.adminReportPage.mockResolvedValue(
      page([report({ id: 7, created_at: "2026-10-25T11:00:00Z" }), report({ id: 8, created_at: "2026-10-25T13:00:00Z" })]),
    );
    await wrapper.findAll(".admin-tabs button")[1]!.trigger("click");
    await flushPromises();
    const reports = wrapper.findAll(".report-item");
    expect(reports.map((item) => item.get(".age").text())).toEqual(["waiting 3 days", "waiting 71 hours"]);
    expect(reports[0]!.get(".overdue-badge").text()).toBe("Overdue");
    expect(reports[1]!.find(".overdue-badge").exists()).toBe(false);

    i18n.global.locale.value = "de";
    await flushPromises();
    expect(wrapper.findAll(".report-item .age").map((node) => node.text())).toEqual([
      "wartet seit 3 Tagen",
      "wartet seit 71 Stunden",
    ]);
    expect(wrapper.get(".overdue-badge").text()).toBe("Überfällig");
  });

  it("lists decided reports in the history, with the 90-day figures and more pages", async () => {
    const root = { id: 1, username: "root", avatar_url: null };
    const decided = Array.from({ length: 50 }, (_, index) =>
      report({
        id: 100 + index,
        status: "dismissed",
        cover_url: null,
        cover_thumb_url: null,
        created_at: "2026-10-20T10:00:00Z",
        resolved_by: root,
        resolved_at: "2026-10-22T12:00:00Z",
      }),
    );
    decided[0] = report({
      id: 100,
      status: "removed",
      removed: true,
      created_by: null,
      details: "Mail from the publisher.",
      cover_url: null,
      cover_thumb_url: null,
      created_at: "2026-10-26T10:00:00Z",
      resolved_by: root,
      resolved_at: "2026-10-26T15:00:00Z",
    });
    decided[1] = report({
      id: 101,
      status: "removed",
      removed: true,
      created_at: "2026-10-26T10:00:00Z",
      resolved_by: null,
      resolved_at: "2026-10-26T10:00:00Z",
      closed_on_arrival: true,
    });
    // Decided by an admin who has since deleted the account: no `resolved_by`, but not closed on arrival.
    decided[3] = report({
      id: 103,
      status: "removed",
      removed: true,
      cover_url: null,
      cover_thumb_url: null,
      created_at: "2026-10-20T10:00:00Z",
      resolved_by: null,
      resolved_at: "2026-10-22T12:00:00Z",
    });
    api.adminSummary.mockResolvedValue(
      summary({ report_stats: { days: 90, resolved: 49, median_hours: 50, max_hours: 80.5, over_target: 1 } }),
    );
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    api.adminReportPage.mockResolvedValueOnce(page(decided, true));
    await buttonIn(wrapper.get(".queue-views"), "Decided").trigger("click");
    await flushPromises();
    expect(api.adminReportPage).toHaveBeenLastCalledWith("resolved", 1);
    expect(wrapper.find("details.takedown").exists()).toBe(false);
    expect(wrapper.get(".report-stats").findAll("dd").map((node) => node.text())).toEqual(["49", "50", "80.5", "1"]);
    expect(wrapper.get(".report-stats").text()).toContain("Took longer than 72 hours");

    const items = wrapper.findAll(".report-item");
    expect(items).toHaveLength(50);
    expect(items[0]!.text()).toContain("Taken down");
    expect(items[0]!.text()).toContain("someone ·");
    expect(items[0]!.get(".decision").text()).toMatch(/^Taken down by root · .+ · after 5 hours$/);
    expect(items[0]!.text()).toContain("Mail from the publisher.");
    expect(items[1]!.get(".decision").text()).toBe("Closed on arrival: the picture was already taken down");
    expect(items[2]!.get(".decision").text()).toMatch(/^Kept by root · .+ · after 50 hours$/);
    expect(items[3]!.get(".decision").text()).toMatch(/^Taken down by someone · .+ · after 50 hours$/);
    expect(items[2]!.findAll("button").map((node) => node.text())).toEqual([]);

    api.adminReportPage.mockResolvedValueOnce(page([report({ id: 150, status: "dismissed", resolved_by: root })], false, 2));
    await buttonIn(wrapper.get(".admin-queue"), "Show more").trigger("click");
    await flushPromises();
    expect(api.adminReportPage).toHaveBeenLastCalledWith("resolved", 2);
    expect(wrapper.findAll(".report-item")).toHaveLength(51);
    expect(wrapper.findAll("button").some((node) => node.text() === "Show more")).toBe(false);

    api.adminReportPage.mockResolvedValueOnce(page([report()]));
    await buttonIn(wrapper.get(".queue-views"), "Open").trigger("click");
    await flushPromises();
    expect(api.adminReportPage).toHaveBeenLastCalledWith("open", 1);
    expect(wrapper.findAll(".report-item")).toHaveLength(1);
  });

  it("lists decided catalog requests without their buttons", async () => {
    const { wrapper } = await mountAdmin();
    api.adminEditRequestPage.mockResolvedValueOnce(
      page([
        editRequest({
          status: "rejected",
          payload: { pages: 393 } as never,
          current: { pages: 393 } as never,
          decided_by: { id: 1, username: "root", avatar_url: null },
          decided_at: "2026-10-28T10:00:00Z",
          decision_note: "The back cover says 392.",
        }),
      ]),
    );
    await buttonIn(wrapper.get(".queue-views"), "Decided").trigger("click");
    await flushPromises();
    expect(api.adminEditRequestPage).toHaveBeenLastCalledWith("resolved", 1);
    const item = wrapper.get(".request-item");
    expect(item.text()).toContain("Rejected");
    expect(item.get(".decision").text()).toMatch(/^Rejected by root · .+ · after 24 hours$/);
    expect(item.text()).toContain("The back cover says 392.");
    // The proposal without "now": a decision made it so or not.
    expect(item.findAll(".field-before")).toHaveLength(0);
    expect(item.findAll("button").map((node) => node.text())).toEqual([]);
  });

  it("takes a picture down by its address and records the notice", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-28T12:00:00Z"));
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    const form = wrapper.get("details.takedown form");
    await form.get("input[name=takedown-url]").setValue("not a link");
    await form.trigger("submit");
    await flushPromises();
    expect(form.get("[role=alert]").text()).toBe("That is not the address of a picture or a book page.");

    const id = "abcdefghijklmnopqrstuvwxyz";
    await form.get("input[name=takedown-url]").setValue(`https://books.example.com/media/${id}/l`);
    await form.get("input[name=received-at]").setValue("2026-10-27T09:30");
    await form.get("select[name=reason]").setValue("offensive");
    await form.get("textarea[name=details]").setValue("  Mail from someone without an account.  ");
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminTakeDown).not.toHaveBeenCalled();
    expect(form.find("[role=alert]").exists()).toBe(false);
    expect(form.get(".takedown-target").text()).toContain("The picture disappears");

    api.adminTakeDown.mockResolvedValue(undefined);
    api.adminSummary.mockClear();
    api.adminReportPage.mockClear();
    expect(buttonIn(form, "Take down for good").attributes("type")).toBe("submit");
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminTakeDown).toHaveBeenCalledWith(id, {
      reason: "offensive",
      details: "Mail from someone without an account.",
      received_at: new Date("2026-10-27T09:30").toISOString(),
    });
    expect(useToast().items.at(-1)?.message).toBe("Picture taken down and the notice recorded");
    expect(api.adminSummary).toHaveBeenCalledTimes(1);
    expect(api.adminReportPage).toHaveBeenCalledWith("open", 1);
    expect((form.get("input[name=takedown-url]").element as HTMLInputElement).value).toBe("");
  });

  it("takes down the cover of a book page, and refuses a notice from the future", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-28T12:00:00Z"));
    api.book.mockResolvedValue(
      bookDetail({ id: 9, title: "Circe", cover_public_id: "zyxwvutsrqponmlkjihgfedcba", cover_url: "/media/zyx/l", cover_thumb_url: "/media/zyx/s" }),
    );
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    const form = wrapper.get("details.takedown form");
    await form.get("input[name=takedown-url]").setValue("https://books.example.com/book/9");
    await form.trigger("submit");
    await flushPromises();
    expect(api.book).toHaveBeenCalledWith(9);
    expect(form.get(".takedown-target").text()).toContain("Circe");

    await form.get("input[name=received-at]").setValue("2026-10-30T09:30");
    expect(buttonIn(form, "Take down for good").attributes("type")).toBe("submit");
    await form.trigger("submit");
    await flushPromises();
    expect(form.get("[role=alert]").text()).toBe("The notice cannot have arrived in the future.");
    expect(api.adminTakeDown).not.toHaveBeenCalled();

    await form.get("input[name=received-at]").setValue("");
    api.adminTakeDown.mockResolvedValue(undefined);
    expect(buttonIn(form, "Take down for good").attributes("type")).toBe("submit");
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminTakeDown).toHaveBeenCalledWith("zyxwvutsrqponmlkjihgfedcba", { reason: "copyright", details: "" });

    api.book.mockResolvedValue(bookDetail({ id: 10, cover_public_id: null }));
    await form.get("input[name=takedown-url]").setValue("/book/10");
    await form.trigger("submit");
    await flushPromises();
    expect(form.get("[role=alert]").text()).toBe("That book shows no picture.");
  });

  it("shows more open reports from the page a decision shortened, listing none twice", async () => {
    const first = Array.from({ length: 50 }, (_, index) =>
      report({ id: 200 + index, media_public_id: `pic${index}`, details: `Report ${200 + index}` }),
    );
    api.adminReportPage.mockResolvedValueOnce(page(first, true));
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(wrapper.findAll(".report-item")).toHaveLength(50);
    api.resolveReport.mockResolvedValue(report({ id: 200, status: "dismissed" }));
    await buttonIn(wrapper.findAll(".report-item")[0]!, "Keep it").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".report-item")).toHaveLength(49);

    // The decision moved everything up by one: the server's page 1 is now the 49 still shown and
    // one more. Page 2 would skip that one.
    const next = report({ id: 250, media_public_id: "pic50", details: "Report 250" });
    api.adminReportPage.mockResolvedValueOnce(page([...first.slice(1), next], true));
    await buttonIn(wrapper.get(".admin-queue"), "Show more").trigger("click");
    await flushPromises();
    expect(api.adminReportPage).toHaveBeenLastCalledWith("open", 1);
    const shown = wrapper.findAll(".report-item .report-details").map((node) => node.text());
    expect(shown).toEqual([...first.slice(1).map((item) => item.details), "Report 250"]);
    expect(new Set(shown).size).toBe(50);
  });

  it("records a notice about a profile picture like one about a cover", async () => {
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    const form = wrapper.get("details.takedown form");
    expect(form.text()).toContain("a cover or a profile picture");
    const id = "avataravataravataravatar22";
    await form.get("input[name=takedown-url]").setValue(`https://books.example.com/media/${id}/a`);
    await form.trigger("submit");
    await flushPromises();
    const target = form.get(".takedown-target");
    expect(target.get(".avatar img").attributes("src")).toBe(`/media/${id}/a`);
    expect(target.text()).toContain("The picture disappears from the profile");

    api.adminTakeDown.mockResolvedValueOnce(undefined);
    expect(buttonIn(form, "Take down for good").attributes("type")).toBe("submit");
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminTakeDown).toHaveBeenCalledTimes(1);
    expect(api.adminTakeDown).toHaveBeenCalledWith(id, expect.objectContaining({ reason: "copyright", details: "" }));
    expect(useToast().items.at(-1)?.message).toBe("Picture taken down and the notice recorded");
    expect(form.findAll("button").map((node) => node.text())).not.toContain("Take down without a record");
  });

  it("shows what each kind of report is about, and words the removal for it", async () => {
    const custom = bookOut({ id: 21, title: "Manual Zine", source: "user", custom: true });
    api.adminReportPage.mockResolvedValue(
      page([
        report({
          id: 31,
          target_kind: "avatar",
          reason: "offensive",
          details: "",
          media_public_id: "gracegracegracegracegrace",
          cover_url: null,
          cover_thumb_url: null,
          avatar_url: GRACE.avatar_url,
          target_user: GRACE,
          book: null,
        }),
        report({ id: 32, target_kind: "work", reason: "copyright", media_public_id: null, cover_url: null, cover_thumb_url: null, book: custom }),
        report({ id: 33, target_kind: "edition", reason: "other", media_public_id: null, cover_url: null, cover_thumb_url: null, book: custom }),
      ]),
    );
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    expect(wrapper.get("#admin-reports").text()).toBe("Reports");
    const [avatar, work, edition] = wrapper.findAll(".report-item");
    expect(avatar!.get(".kind-badge").text()).toBe("Profile picture");
    expect(avatar!.text()).toContain("Profile picture of grace");
    expect(avatar!.get(".avatar img").attributes("src")).toBe(GRACE.avatar_url);
    expect(work!.get(".kind-badge").text()).toBe("Book added by a reader");
    expect(work!.get("a").text()).toBe("Manual Zine");
    expect(edition!.get(".kind-badge").text()).toBe("Edition added by a reader");

    await buttonIn(avatar!, "Take down").trigger("click");
    expect(avatar!.text()).toContain("The picture disappears from the profile and everywhere else");
    await buttonIn(avatar!, "Cancel").trigger("click");

    await buttonIn(work!, "Remove the text").trigger("click");
    expect(api.resolveReport).not.toHaveBeenCalled();
    expect(work!.text()).toContain("The title becomes “[removed]”");
    api.resolveReport.mockResolvedValue(report({ id: 32, target_kind: "work", status: "removed" }));
    // The other reports of the book are not told apart by the list: the queue is read again.
    api.adminReportPage.mockResolvedValue(page([report({ id: 31, target_kind: "avatar", target_user: GRACE, book: null })]));
    await buttonIn(work!, "Remove for good").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenCalledWith(32, "remove");
    expect(useToast().items.at(-1)?.message).toBe("Text removed");
    expect(api.adminReportPage).toHaveBeenLastCalledWith("open", 1);
    expect(wrapper.findAll(".report-item")).toHaveLength(1);

    i18n.global.locale.value = "de";
    await flushPromises();
    expect(wrapper.get(".report-item .kind-badge").text()).toBe("Profilbild");
  });

  it("offers only closing a report of something gone, and reads the queue again on a 409", async () => {
    const circe = bookOut({ id: 9, title: "Circe" });
    const avatarGone = report({
      id: 51,
      target_kind: "avatar",
      media_public_id: null,
      cover_url: null,
      cover_thumb_url: null,
      avatar_url: null,
      target_user: GRACE,
      book: null,
      gone: true,
    });
    const merged = report({ id: 52, target_kind: "work", media_public_id: null, cover_url: null, cover_thumb_url: null, book: circe, gone: true });
    const avatar = report({ id: 53, target_kind: "avatar", media_public_id: "gracegracegracegracegrace", cover_url: null, cover_thumb_url: null, avatar_url: GRACE.avatar_url, target_user: GRACE, book: null });
    api.adminReportPage.mockResolvedValue(page([avatarGone, merged, avatar]));
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    const [gonePicture, mergedBook, live] = wrapper.findAll(".report-item");
    expect(gonePicture!.text()).toContain("No longer online: its owner replaced or deleted the picture");
    expect(mergedBook!.text()).toContain("Merged into this book since it was reported");
    for (const item of [gonePicture!, mergedBook!]) {
      expect(item.findAll("button").map((node) => node.text())).toEqual(["Close the report"]);
    }

    // Dismissing closes the owner's other gone reports too: the queue is read again.
    api.resolveReport.mockResolvedValue({ ...avatarGone, status: "dismissed" });
    api.adminReportPage.mockResolvedValue(page([merged, avatar]));
    await buttonIn(gonePicture!, "Close the report").trigger("click");
    await flushPromises();
    expect(api.resolveReport).toHaveBeenCalledWith(51, "dismiss");
    expect(useToast().items.at(-1)?.message).toBe("Report closed, nothing was removed");
    expect(wrapper.findAll(".report-item")).toHaveLength(2);

    // The owner replaced the picture after the list was read: the takedown is refused, and the
    // queue read again shows the report as gone.
    const liveItem = wrapper.findAll(".report-item")[1]!;
    await buttonIn(liveItem, "Take down").trigger("click");
    api.resolveReport.mockRejectedValue(
      new ApiError(i18n.global.t("errors.pictureGone"), 409, "The reported picture is no longer online"),
    );
    api.adminReportPage.mockResolvedValue(page([merged, { ...avatar, media_public_id: null, avatar_url: null, gone: true }]));
    await buttonIn(liveItem, "Take down for good").trigger("click");
    await flushPromises();
    expect(useToast().items.at(-1)?.message).toBe(
      "The reported picture is no longer online: its owner replaced or deleted it. Close the report.",
    );
    expect(api.adminReportPage).toHaveBeenLastCalledWith("open", 1);
    const [, refreshed] = wrapper.findAll(".report-item");
    expect(refreshed!.findAll("button").map((node) => node.text())).toEqual(["Close the report"]);
  });

  it("names an emailed notice and a removed book's text in the history", async () => {
    const root = { id: 1, username: "root", avatar_url: null };
    const decided = [
      report({
        id: 40,
        via: "notice",
        created_by: null,
        status: "removed",
        removed: true,
        cover_url: null,
        cover_thumb_url: null,
        created_at: "2026-10-26T10:00:00Z",
        resolved_by: root,
        resolved_at: "2026-10-26T12:00:00Z",
      }),
      report({
        id: 41,
        target_kind: "work",
        status: "removed",
        media_public_id: null,
        cover_url: null,
        cover_thumb_url: null,
        book: bookOut({ id: 21, title: "[removed]", source: "user", custom: true }),
        created_at: "2026-10-26T10:00:00Z",
        resolved_by: root,
        resolved_at: "2026-10-26T13:00:00Z",
      }),
      report({
        id: 42,
        target_kind: "work",
        status: "dismissed",
        media_public_id: null,
        cover_url: null,
        cover_thumb_url: null,
        gone: true,
        created_at: "2026-10-26T10:00:00Z",
        resolved_by: root,
        resolved_at: "2026-10-26T11:00:00Z",
      }),
    ];
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    api.adminReportPage.mockResolvedValueOnce(page(decided));
    await buttonIn(wrapper.get(".queue-views"), "Decided").trigger("click");
    await flushPromises();
    const [notice, work, merged] = wrapper.findAll(".report-item");
    // A report of a book merged away was closed, not kept.
    expect(merged!.get(".decision").text()).toMatch(/^Closed by root · .+ · after an hour$/);
    expect(notice!.get(".admin-item-head").text()).toMatch(/Emailed notice · received .+/);
    expect(notice!.get(".admin-item-head").text()).not.toContain("someone");
    expect(notice!.get(".decision").text()).toMatch(/^Taken down by root · .+ · after 2 hours$/);
    expect(work!.get(".status-removed").text()).toBe("Removed");
    expect(work!.get(".decision").text()).toMatch(/^Removed by root · .+ · after 3 hours$/);
  });

  it("suspends an account by name with a second tap, and lifts a suspension", async () => {
    const suspended = (overrides: Partial<AdminUserOut>): AdminUserOut => ({
      id: 5,
      username: "mallory",
      avatar_url: null,
      is_admin: false,
      created_at: "2026-10-01T10:00:00Z",
      suspended_at: "2026-10-28T09:00:00Z",
      ...overrides,
    });
    api.adminSuspendedUsers.mockResolvedValue({ items: [suspended({})] });
    const { wrapper, router } = await mountAdmin();
    await wrapper.findAll(".admin-tabs button").find((node) => node.text() === "Accounts")!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.query).toEqual({ tab: "users" });
    expect(api.adminSuspendedUsers).toHaveBeenCalledTimes(1);
    expect(wrapper.get(".user-row").text()).toContain("mallory");
    expect(wrapper.get(".user-row").text()).toContain("suspended");

    const form = wrapper.get("form.suspend-form");
    await form.get("input[name=suspend-username]").setValue("  root ");
    await form.trigger("submit");
    expect(api.adminSuspendUser).not.toHaveBeenCalled();
    expect(form.text()).toContain("root is signed out everywhere");
    const refusal = "You cannot suspend your own account";
    api.adminSuspendUser.mockRejectedValueOnce(new ApiError(refusal, 400, refusal));
    expect(buttonIn(form, "Suspend root").attributes("type")).toBe("submit");
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminSuspendUser).toHaveBeenCalledWith("root");
    expect(form.get("[role=alert]").text()).toBe(refusal);

    await form.get("input[name=suspend-username]").setValue("eve");
    await form.get("input[name=suspend-username]").trigger("input");
    await form.trigger("submit");
    api.adminSuspendUser.mockResolvedValueOnce(suspended({ id: 6, username: "eve", is_admin: true }));
    await form.trigger("submit");
    await flushPromises();
    expect(api.adminSuspendUser).toHaveBeenLastCalledWith("eve");
    expect(useToast().items.at(-1)?.message).toBe("eve is suspended");
    expect(wrapper.findAll(".user-row").map((row) => row.get("strong").text())).toEqual(["eve", "mallory"]);
    expect(wrapper.findAll(".user-row")[0]!.text()).toContain("Admin");
    expect((form.get("input[name=suspend-username]").element as HTMLInputElement).value).toBe("");

    api.adminUnsuspendUser.mockResolvedValueOnce(suspended({ suspended_at: null }));
    await buttonIn(wrapper.findAll(".user-row")[1]!, "Lift suspension").trigger("click");
    await flushPromises();
    expect(api.adminUnsuspendUser).toHaveBeenCalledWith("mallory");
    expect(useToast().items.at(-1)?.message).toBe("mallory can sign in again");
    expect(wrapper.findAll(".user-row")).toHaveLength(1);
  });

  it("words the server's refusal of an arrival time ahead of its clock", async () => {
    const { wrapper } = await mountAdmin("/admin?tab=reports");
    i18n.global.locale.value = "de";
    const form = wrapper.get("details.takedown form");
    await form.get("input[name=takedown-url]").setValue("abcdefghijklmnopqrstuvwxyz");
    await form.trigger("submit");
    await flushPromises();
    // This device's clock is behind the server's, so its own check passed.
    const refusal = "Input should be in the past";
    api.adminTakeDown.mockRejectedValueOnce(new ApiError(refusal, 400, refusal));
    await form.trigger("submit");
    await flushPromises();
    expect(form.get("[role=alert]").text()).toBe("Die Meldung kann nicht in der Zukunft eingegangen sein.");
    expect(buttonIn(form, "Endgültig entfernen")).toBeTruthy();
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
