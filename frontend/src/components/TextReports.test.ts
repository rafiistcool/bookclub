import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { activityItem, bookOut, post, user } from "../test/fixtures";
import type { QuoteOut } from "../types";

/**
 * Reports on texts (#55, Q11 option C): a post, a quote or a take another
 * reader wrote is reported through `ReportSheet`, from where it shows.
 */
const api: Record<string, ReturnType<typeof vi.fn>> = {
  reportPost: vi.fn(),
  reportQuote: vi.fn(),
  reportTake: vi.fn(),
  reportCover: vi.fn(),
  quotes: vi.fn(),
  activity: vi.fn(),
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

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import ActivityFeed from "./ActivityFeed.vue";
import DiaryEntry from "./DiaryEntry.vue";
import QuoteList from "./QuoteList.vue";
import ReportSheet, { isTextTarget, type ReportTarget } from "./ReportSheet.vue";

function reasonsOf(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAll('input[name="reason"]').map((node) => node.attributes("value"));
}

async function send(wrapper: Pick<ReturnType<typeof mount>, "get">, reason: string) {
  await wrapper.get(`input[value="${reason}"]`).setValue(true);
  await wrapper.get("form").trigger("submit");
  await flushPromises();
}

function quote(overrides: Partial<QuoteOut> = {}): QuoteOut {
  return {
    id: 3,
    book: bookOut({ id: 9, title: "Circe" }),
    edition_id: null,
    author: "grace",
    mine: false,
    body: "Witches are not gentle.",
    page: null,
    visibility: "audience",
    created_at: "2026-10-01T10:00:00Z",
    created_label: "Thu 1 Oct 2026",
    ...overrides,
  };
}

describe("reports on texts", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
    useSession().user = user({ username: "ada" });
  });

  it("sends a post, a quote and a take to their own routes, without the wrong-book reason", async () => {
    api.reportPost.mockResolvedValue({ id: 1, status: "open" });
    const postSheet = mount(ReportSheet, { props: { target: { kind: "post", postId: 5, author: "grace" } } });
    expect(postSheet.text()).toContain("Report this post");
    expect(postSheet.text()).toContain("Post by grace");
    expect(reasonsOf(postSheet)).toEqual(["offensive", "copyright", "other"]);
    await send(postSheet, "offensive");
    expect(api.reportPost).toHaveBeenCalledWith(5, { reason: "offensive", details: null });
    expect(useToast().items[0]?.message).toBe("Thanks. An admin will look at it.");

    api.reportQuote.mockResolvedValue({ id: 2, status: "open" });
    const quoteSheet = mount(ReportSheet, { props: { target: { kind: "quote", quoteId: 3, author: "grace" } } });
    expect(quoteSheet.text()).toContain("Quote saved by grace");
    await send(quoteSheet, "copyright");
    expect(api.reportQuote).toHaveBeenCalledWith(3, { reason: "copyright", details: null });

    // A take already removed by an admin: the report is closed on arrival.
    api.reportTake.mockResolvedValue({ id: 3, status: "removed" });
    i18n.global.locale.value = "de";
    const takeSheet = mount(ReportSheet, { props: { target: { kind: "take", workId: 9, author: "grace" } } });
    expect(takeSheet.text()).toContain("Diesen Satz melden");
    expect(takeSheet.text()).toContain("Satz von grace");
    await send(takeSheet, "other");
    expect(api.reportTake).toHaveBeenCalledWith(9, "grace", { reason: "other", details: null });
    expect(useToast().items.at(-1)?.message).toBe("Ein Admin hat diesen Text schon entfernt. Trotzdem danke.");
    expect(api.reportCover).not.toHaveBeenCalled();
  });

  it("knows a text target from a picture or a book", () => {
    const targets: ReportTarget[] = [
      { kind: "post", postId: 1, author: "a" },
      { kind: "quote", quoteId: 1, author: "a" },
      { kind: "take", workId: 1, author: "a" },
      { kind: "cover", publicId: "x", title: "t" },
      { kind: "work", workId: 1, title: "t" },
    ];
    expect(targets.map(isTextTarget)).toEqual([true, true, true, false, false]);
  });

  it("offers a report behind someone else's post's “…”, not behind one's own or a tombstone", async () => {
    api.reportPost.mockResolvedValue({ id: 1, status: "open" });
    const mountEntry = (entry: ReturnType<typeof post>) =>
      mount(DiaryEntry, {
        props: { entry, myProgress: null, myStatus: null },
        global: { stubs: { Avatar: true, SpoilerControl: true } },
        attachTo: document.body,
      });
    const theirs = mountEntry(post({ id: 8, author: "grace", body: "Rude words" }));
    await theirs.get(".entry-more").trigger("click");
    const row = document.querySelector<HTMLButtonElement>(".post-report");
    expect(row?.textContent?.trim()).toBe("Report");
    expect(document.querySelector(".post-edit")).toBeNull();
    expect(document.querySelector(".post-delete")).toBeNull();
    row!.click();
    await flushPromises();
    expect(document.body.textContent).toContain("Report this post");
    expect(document.body.textContent).toContain("Post by grace");
    theirs.unmount();

    const mine = mountEntry(post({ id: 9, author: "ada", mine: true, body: "Mine" }));
    await mine.get(".entry-more").trigger("click");
    expect(document.querySelector(".post-report")).toBeNull();
    mine.unmount();

    const tombstone = mountEntry(post({ id: 10, author: "grace", deleted: true }));
    expect(tombstone.find(".entry-more").exists()).toBe(false);
    tombstone.unmount();
  });

  it("offers a report on someone else's quote only", async () => {
    api.quotes.mockResolvedValue({ items: [quote(), quote({ id: 4, author: "ada", mine: true, body: "My line" })] });
    const wrapper = mount(QuoteList, { props: { workId: 9 }, global: { stubs: { BookCover: true } } });
    await flushPromises();
    const reports = wrapper.findAll(".quote-report");
    expect(reports).toHaveLength(1);
    await reports[0]!.trigger("click");
    api.reportQuote.mockResolvedValue({ id: 1, status: "open" });
    await send(wrapper.findComponent(ReportSheet), "copyright");
    expect(api.reportQuote).toHaveBeenCalledWith(3, { reason: "copyright", details: null });
  });

  it("offers a report on another reader's take in the feed", async () => {
    const circe = bookOut({ id: 9, title: "Circe" });
    api.activity.mockResolvedValue({
      items: [
        activityItem({ id: 3, kind: "shelf_finished", actor: "grace", book: circe, payload: { rating: 4, take: "Meh" } }),
        activityItem({ id: 2, kind: "shelf_finished", actor: "ada", book: circe, payload: { take: "Mine" } }),
        activityItem({ id: 1, kind: "shelf_finished", actor: "grace", book: circe, payload: { rating: 5 } }),
      ],
      has_more: false,
      timezone: "UTC",
    });
    const wrapper = mount(ActivityFeed, { global: { stubs: { BookCover: true, Avatar: true } } });
    await flushPromises();
    const reports = wrapper.findAll(".activity-report");
    expect(reports).toHaveLength(1);
    expect(reports[0]!.attributes("aria-label")).toBe("Report grace's take");
    await reports[0]!.trigger("click");
    api.reportTake.mockResolvedValue({ id: 1, status: "open" });
    await send(wrapper.findComponent(ReportSheet), "offensive");
    expect(api.reportTake).toHaveBeenCalledWith(9, "grace", { reason: "offensive", details: null });
  });
});
