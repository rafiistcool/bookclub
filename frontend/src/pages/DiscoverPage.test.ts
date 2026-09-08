import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SearchHit, SearchPage } from "../types";

const search = vi.fn();
const trending = vi.fn();
const subject = vi.fn();
const replace = vi.fn();
const routeQuery = { q: "" as string, subject: "" as string };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.status = status;
    }
  },
  api: {
    search: (...args: unknown[]) => search(...args),
    trending: (...args: unknown[]) => trending(...args),
    subject: (...args: unknown[]) => subject(...args),
  },
}));

vi.mock("vue-router", () => ({
  useRoute: () => ({ query: routeQuery }),
  useRouter: () => ({ replace }),
}));

import { ApiError } from "../api/client";
import DiscoverPage from "./DiscoverPage.vue";

function hit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    ol_work_key: "/works/OL1W",
    title: "Circe",
    authors: "Madeline Miller",
    cover_id: 123,
    year: 2018,
    on_shelf: null,
    shelf_id: null,
    ...overrides,
  };
}

function page(items: SearchHit[], extras: Partial<SearchPage> = {}): SearchPage {
  return { items, page: 1, has_more: false, ...extras };
}

async function mountDiscover() {
  const wrapper = mount(DiscoverPage, {
    global: { plugins: [createPinia()] },
  });
  await flushPromises();
  return wrapper;
}

describe("DiscoverPage", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    routeQuery.q = "";
    routeQuery.subject = "";
    search.mockReset();
    trending.mockReset();
    subject.mockReset();
    replace.mockReset();
    trending.mockResolvedValue(page([hit({ ol_work_key: "/works/OL7W", title: "Atomic Habits" })]));
    subject.mockResolvedValue(page([hit()]));
  });

  it("does not load browse shelves when a query is already in the URL", async () => {
    routeQuery.q = "circe";
    search.mockResolvedValue(page([hit()]));
    const wrapper = await mountDiscover();
    expect(trending).not.toHaveBeenCalled();
    expect(subject).not.toHaveBeenCalled();
    expect(search).toHaveBeenCalled();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.find("input").attributes("placeholder")).toBe("Title, author, or ISBN");
  });

  it("shows a short-query state for It and does not call search", async () => {
    const wrapper = await mountDiscover();
    await wrapper.get("input").setValue("it");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(search).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("That search is too short");
    expect(wrapper.text()).toContain("3 characters");
    expect(wrapper.text()).toContain("Add your own book");
  });

  it("keeps visible hits when a later page fails", async () => {
    routeQuery.q = "circe";
    search
      .mockResolvedValueOnce(page([hit()], { has_more: true }))
      .mockRejectedValueOnce(
        new ApiError("Could not search the library right now. Try again.", 502, "unavailable"),
      );
    const wrapper = await mountDiscover();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).toContain("Couldn't load more");
    expect(wrapper.text()).not.toContain("The library is unavailable");
  });

  it("distinguishes a rate-limit from a miss", async () => {
    routeQuery.q = "circe";
    search.mockRejectedValue(
      new ApiError("The library is busy. Try again in 20 seconds.", 429, "rate"),
    );
    const wrapper = await mountDiscover();
    expect(wrapper.text()).toContain("The library is busy");
    expect(wrapper.text()).toContain("20 seconds");
    expect(wrapper.text()).not.toContain("Nothing matched");
  });

  it("offers to clear the subject filter and add a book on a true miss", async () => {
    routeQuery.q = "circe";
    routeQuery.subject = "fantasy";
    search.mockResolvedValue(page([]));
    const wrapper = await mountDiscover();
    expect(wrapper.text()).toContain("Nothing matched");
    expect(wrapper.text()).toContain("Clear Fantasy");
    expect(wrapper.text()).toContain("Add your own book");
    expect(wrapper.text()).toContain("ISBN");
  });

  it("retries a failed subject shelf instead of only offering search", async () => {
    subject.mockRejectedValueOnce(new Error("down"));
    const wrapper = await mountDiscover();
    expect(wrapper.text()).toContain("Try again");
    expect(wrapper.text()).toContain("Search it instead");
    subject.mockResolvedValue(page([hit({ title: "Dune" })]));
    const retry = wrapper.findAll("button").find((btn) => btn.text() === "Try again");
    await retry!.trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");
  });
});
