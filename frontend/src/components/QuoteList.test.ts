import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, user } from "../test/fixtures";
import type { QuoteOut } from "../types";

const api = { quotes: vi.fn(), addQuote: vi.fn(), editQuote: vi.fn(), deleteQuote: vi.fn() };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import QuoteList from "./QuoteList.vue";

function quote(overrides: Partial<QuoteOut> = {}): QuoteOut {
  return {
    id: 1,
    book: bookOut({ id: 9, title: "Circe" }),
    edition_id: null,
    author: "ada",
    mine: true,
    body: "Witches are not gentle.",
    page: 12,
    visibility: "audience",
    created_at: "2026-10-01T10:00:00Z",
    created_label: "Thu 1 Oct 2026",
    ...overrides,
  };
}

async function mountList(props: Record<string, unknown>) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user();
  const wrapper = mount(QuoteList, { props, global: { plugins: [pinia, i18n], stubs: { BookCover: true } } });
  await flushPromises();
  return wrapper;
}

describe("QuoteList", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  it("keeps a quote from a book, for everyone or only for me", async () => {
    api.quotes.mockResolvedValue({ items: [] });
    const wrapper = await mountList({ workId: 9 });
    expect(api.quotes).toHaveBeenCalledWith({ work: 9, username: undefined });
    api.addQuote.mockResolvedValue(quote({ id: 2, visibility: "only_me", page: 40 }));
    await wrapper.get(".quote-compose textarea").setValue("  Witches are not gentle. ");
    await wrapper.get(".quote-page input").setValue("40");
    await wrapper.get(".quote-only-me input").setValue(true);
    await wrapper.get(".quote-compose").trigger("submit");
    await flushPromises();
    expect(api.addQuote).toHaveBeenCalledWith({ work_id: 9, body: "Witches are not gentle.", page: 40, visibility: "only_me" });
    expect(wrapper.get(".only-me-badge").text()).toBe("Only you");
    expect(wrapper.get(".quote-meta").text()).toContain("p. 40");
  });

  it("lists a reader's quotes with their books, and edits and deletes only mine", async () => {
    api.quotes.mockResolvedValue({ items: [quote(), quote({ id: 3, mine: false, author: "grace", body: "Other" })] });
    const wrapper = await mountList({ username: "ada", showBook: true });
    expect(api.quotes).toHaveBeenCalledWith({ work: undefined, username: "ada", limit: 500 });
    expect(wrapper.find(".quotes-capped").exists()).toBe(false);
    expect(wrapper.find(".quote-compose").exists()).toBe(false);
    expect(wrapper.findAll(".quote-book")[0]!.attributes("href")).toBe("/book/9");
    expect(wrapper.findAll(".quote")[1]!.find(".quote-actions").exists()).toBe(false);

    api.editQuote.mockResolvedValue(quote({ visibility: "only_me", page: null }));
    await wrapper.findAll(".quote-actions .text-btn")[0]!.trigger("click");
    await wrapper.get(".quote-edit .quote-page input").setValue("");
    await wrapper.get(".quote-edit .quote-only-me input").setValue(true);
    await wrapper.get(".quote-edit").trigger("submit");
    await flushPromises();
    expect(api.editQuote).toHaveBeenCalledWith(1, { body: "Witches are not gentle.", page: null, visibility: "only_me" });

    api.deleteQuote.mockResolvedValue(undefined);
    const del = () => wrapper.findAll(".quote-actions .text-btn")[1]!;
    await del().trigger("click");
    expect(api.deleteQuote).not.toHaveBeenCalled();
    await del().trigger("click");
    await flushPromises();
    expect(api.deleteQuote).toHaveBeenCalledWith(1);
    expect(wrapper.findAll(".quote")).toHaveLength(1);
  });

  it("asks for the server's maximum on a reader's list and says when it was cut", async () => {
    const many = Array.from({ length: 500 }, (_, index) => quote({ id: 1000 - index }));
    api.quotes.mockResolvedValue({ items: many });
    const wrapper = await mountList({ username: "ada", showBook: true });
    expect(wrapper.findAll(".quote")).toHaveLength(500);
    expect(wrapper.get(".quotes-capped").text()).toBe(
      "Showing the newest 500 quotes. Older ones are kept, but this list cannot show them yet.",
    );

    // A book's quotes keep the server's default.
    api.quotes.mockResolvedValue({ items: [] });
    await mountList({ workId: 9 });
    expect(api.quotes).toHaveBeenLastCalledWith({ work: 9, username: undefined });
  });
});
