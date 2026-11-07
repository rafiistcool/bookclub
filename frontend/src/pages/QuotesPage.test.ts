import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { describe, expect, it, vi } from "vitest";
import { bookOut, user } from "../test/fixtures";

const quotes = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { quotes: (...args: unknown[]) => quotes(...args) },
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import QuotesPage from "./QuotesPage.vue";

describe("QuotesPage", () => {
  it("shows the count beside the title, with no second visible heading", async () => {
    i18n.global.locale.value = "en";
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ username: "ada" });
    const item = (id: number) => ({
      id, book: bookOut({ id: 9, title: "Circe" }), edition_id: null, author: "ada", mine: true, body: "Line",
      page: null, visibility: "audience", created_at: "2026-10-01T10:00:00Z", created_label: "Thu 1 Oct 2026",
    });
    quotes.mockResolvedValue({ items: [item(1), item(2), item(3)] });
    const wrapper = mount(QuotesPage, {
      global: { plugins: [pinia, i18n], stubs: { BookCover: true, RouterLink: { template: "<a><slot /></a>" } } },
    });
    await flushPromises();
    expect(wrapper.get(".page-head h1").text()).toBe("Quotes");
    expect(wrapper.get(".page-head .quotes-count").text()).toBe("3");
    expect(wrapper.get("h2").classes()).toContain("visually-hidden");
  });
});
