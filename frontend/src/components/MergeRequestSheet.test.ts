import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookDetail, searchHit } from "../test/fixtures";

const api = { search: vi.fn(), book: vi.fn(), requestMerge: vi.fn() };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { useToast } from "../stores/toast";
import MergeRequestSheet from "./MergeRequestSheet.vue";

function mountSheet() {
  return mount(MergeRequestSheet, {
    props: { workId: 9, title: "Circe" },
    global: { stubs: { teleport: true, BookCover: true } },
  });
}

describe("MergeRequestSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const fn of Object.values(api)) fn.mockReset();
  });

  it("finds the other entry by search, leaves out this one, and sends the request", async () => {
    api.search.mockResolvedValue({
      items: [searchHit({ id: 9, title: "Circe" }), searchHit({ id: 31, title: "Circe (Roman)", authors: "Madeline Miller", year: 2019 })],
      has_more: false,
    });
    api.requestMerge.mockResolvedValue({ request_id: 5 });
    const wrapper = mountSheet();
    await wrapper.get("input[name=merge-query]").setValue("circe miller");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.search).toHaveBeenCalledWith({ q: "circe miller", sort: "relevance", limit: 8 });
    const hits = wrapper.findAll(".merge-hit");
    expect(hits).toHaveLength(1);
    expect(hits[0].text()).toContain("Circe (Roman)");

    await hits[0].trigger("click");
    expect(wrapper.get(".merge-chosen").text()).toContain("“Circe” is the same book as “Circe (Roman)”.");
    await wrapper.get("textarea").setValue("  Same ISBN on the back.  ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.requestMerge).toHaveBeenCalledWith(9, 31, "Same ISBN on the back.");
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("opens a pasted link directly", async () => {
    api.book.mockResolvedValue(bookDetail({ id: 31, title: "Ich bin Circe" }));
    const wrapper = mountSheet();
    await wrapper.get("input[name=merge-query]").setValue("https://bookclub.example/book/31");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.search).not.toHaveBeenCalled();
    expect(api.book).toHaveBeenCalledWith(31);
    expect(wrapper.get(".merge-hit").text()).toContain("Ich bin Circe");
  });
});
