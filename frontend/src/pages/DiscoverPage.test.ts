import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SearchHit, SearchPage } from "../types";

const search = vi.fn();
const trending = vi.fn();
const subject = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number, _detail?: unknown) {
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

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import DiscoverPage from "./DiscoverPage.vue";
import { resetDiscoverBrowseCache } from "./discoverCache";

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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const bookTileStub = {
  props: ["title"],
  template: "<div class='tile'>{{ title }}</div>",
};

async function mountDiscover(initial = "/discover") {
  const router = createRouter({
    history: createMemoryHistory(),
    scrollBehavior() {
      return { top: 0 };
    },
    routes: [
      {
        path: "/",
        component: { template: "<router-view />" },
        children: [{ path: "discover", component: DiscoverPage }],
      },
    ],
  });
  await router.push(initial);
  await router.isReady();
  const wrapper = mount(
    { template: "<router-view />" },
    {
      global: {
        plugins: [createPinia(), router, i18n],
        stubs: { BookTile: bookTileStub, AddBookSheet: true },
      },
    },
  );
  await flushPromises();
  return { wrapper, router };
}

async function submitQuery(wrapper: VueWrapper, q: string) {
  const input = wrapper.get("input[type='search']");
  await input.setValue(q);
  await wrapper.get("form").trigger("submit");
  await nextTick();
}

describe("DiscoverPage", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    resetDiscoverBrowseCache();
    search.mockReset();
    trending.mockReset();
    subject.mockReset();
    trending.mockResolvedValue(page([hit({ ol_work_key: "/works/OL7W", title: "Atomic Habits" })]));
    subject.mockResolvedValue(page([hit()]));
  });

  it("does not load browse shelves when a query is already in the URL", async () => {
    search.mockResolvedValue(page([hit()]));
    const { wrapper } = await mountDiscover("/discover?q=circe");
    expect(trending).not.toHaveBeenCalled();
    expect(subject).not.toHaveBeenCalled();
    expect(search).toHaveBeenCalled();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.find("input").attributes("placeholder")).toBe("Title, author, or ISBN");
  });

  it("shows a short-query state for It and does not call search", async () => {
    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "it");
    await flushPromises();
    expect(search).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("That search is too short");
    expect(wrapper.text()).toContain("3 characters");
    expect(wrapper.text()).toContain("Add your own book");
  });

  it("keeps visible hits when a later page fails", async () => {
    search
      .mockResolvedValueOnce(page([hit()], { has_more: true }))
      .mockRejectedValueOnce(
        new ApiError("Could not search the library right now. Try again.", 502, "unavailable"),
      );
    const { wrapper } = await mountDiscover("/discover?q=circe");
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).toContain("Couldn't load more");
    expect(wrapper.text()).not.toContain("The library is unavailable");
  });

  it("distinguishes a rate-limit from a miss", async () => {
    search.mockRejectedValue(
      new ApiError("The library is busy. Try again in 20 seconds.", 429, "rate"),
    );
    const { wrapper } = await mountDiscover("/discover?q=circe");
    expect(wrapper.text()).toContain("The library is busy");
    expect(wrapper.text()).toContain("20 seconds");
    expect(wrapper.text()).not.toContain("Nothing matched");
  });

  it("offers to clear the subject filter and add a book on a true miss", async () => {
    search.mockResolvedValue(page([]));
    const { wrapper } = await mountDiscover("/discover?q=circe&subject=fantasy");
    expect(wrapper.text()).toContain("Nothing matched");
    expect(wrapper.text()).toContain("Clear Fantasy");
    expect(wrapper.text()).toContain("Add your own book");
    expect(wrapper.text()).toContain("ISBN");
  });

  it("starts two subject shelves before waiting on the rest", async () => {
    const fiction = deferred<SearchPage>();
    const scifi = deferred<SearchPage>();
    const mystery = deferred<SearchPage>();
    subject.mockImplementation((name: string) => {
      if (name === "fiction") return fiction.promise;
      if (name === "science_fiction") return scifi.promise;
      if (name === "mystery") return mystery.promise;
      return Promise.resolve(page([]));
    });
    await mountDiscover();
    expect(subject.mock.calls.map((call) => call[0])).toEqual(["fiction", "science_fiction"]);
    fiction.resolve(page([hit({ title: "Circe" })]));
    await flushPromises();
    expect(subject.mock.calls.map((call) => call[0])).toEqual([
      "fiction",
      "science_fiction",
      "mystery",
    ]);
  });

  it("keeps browse tiles visible when remounting Discover", async () => {
    const { wrapper } = await mountDiscover();
    expect(wrapper.text()).toContain("Atomic Habits");
    wrapper.unmount();

    const refresh = deferred<SearchPage>();
    trending.mockReset();
    trending.mockImplementation(() => refresh.promise);
    const { wrapper: again } = await mountDiscover();
    expect(again.text()).toContain("Atomic Habits");
    expect(again.text()).not.toContain("Still loading shelves");
    refresh.resolve(page([hit({ ol_work_key: "/works/OL7W", title: "Atomic Habits" })]));
    await flushPromises();
    expect(again.text()).toContain("Atomic Habits");
  });

  it("retries a failed subject shelf instead of only offering search", async () => {
    subject.mockRejectedValueOnce(new Error("down"));
    const { wrapper } = await mountDiscover();
    expect(wrapper.text()).toContain("Try again");
    expect(wrapper.text()).toContain("Search it instead");
    subject.mockResolvedValue(page([hit({ title: "Dune" })]));
    const retry = wrapper.findAll("button").find((btn) => btn.text() === "Try again");
    await retry!.trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");
  });
});

describe("DiscoverPage search results", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    resetDiscoverBrowseCache();
    search.mockReset();
    trending.mockReset();
    subject.mockReset();
    trending.mockResolvedValue(page([hit({ ol_work_key: "/works/OLT", title: "Trending Book" })]));
    subject.mockResolvedValue(page([]));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not empty visible results after they land while the route catches up", async () => {
    const first = deferred<SearchPage>();
    search.mockImplementationOnce(() => first.promise);

    const { wrapper, router } = await mountDiscover();
    const titles = () => wrapper.findAll(".tile").map((n) => n.text());

    await submitQuery(wrapper, "dune");
    expect(search).toHaveBeenCalledTimes(1);
    expect(titles()).not.toContain("Dune");

    first.resolve(page([hit({ ol_work_key: "/works/OL1W", title: "Dune" })]));
    await flushPromises();
    await nextTick();

    const afterFirst = titles();
    expect(afterFirst).toContain("Dune");

    // Let vue-router finish replace({ path, query }) and any follow-up watchers.
    await flushPromises();
    await nextTick();
    await router.isReady();
    await flushPromises();

    const afterRoute = titles();
    const pageOneCalls = search.mock.calls.filter((call) => {
      const params = (call[0] ?? {}) as { page?: number; q?: string };
      return (params.page ?? 1) === 1;
    });

    expect(afterRoute).toContain("Dune");
    expect(afterRoute).toEqual(afterFirst);
    expect(pageOneCalls).toHaveLength(1);
    expect(trending).toHaveBeenCalledTimes(1);
  });

  it("keeps the current result list visible when a newer search is in flight", async () => {
    const dune = deferred<SearchPage>();
    const circe = deferred<SearchPage>();
    search.mockImplementationOnce(() => dune.promise).mockImplementationOnce(() => circe.promise);

    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "dune");
    dune.resolve(page([hit({ ol_work_key: "/works/OL1W", title: "Dune" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");

    await submitQuery(wrapper, "circe");
    await nextTick();
    expect(wrapper.text()).toContain("Dune");
    expect(wrapper.text()).not.toContain("Nothing matched");

    circe.resolve(page([hit({ ol_work_key: "/works/OL2W", title: "Circe" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).not.toContain("Dune");
  });

  it("does not flash a previous search after Clear", async () => {
    const dune = deferred<SearchPage>();
    const circe = deferred<SearchPage>();
    search.mockImplementationOnce(() => dune.promise).mockImplementationOnce(() => circe.promise);

    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "dune");
    dune.resolve(page([hit({ ol_work_key: "/works/OL1W", title: "Dune" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");

    await wrapper.get("button.text-btn").trigger("click");
    await nextTick();
    expect(wrapper.text()).not.toContain("Dune");

    await submitQuery(wrapper, "circe");
    await nextTick();
    expect(wrapper.text()).not.toContain("Dune");
    expect(wrapper.text()).not.toContain("Circe");

    circe.resolve(page([hit({ ol_work_key: "/works/OL2W", title: "Circe" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
  });

  it("still shows the empty state when a reset search returns nothing", async () => {
    search.mockResolvedValue(page([]));
    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "xyzzy");
    await flushPromises();
    expect(wrapper.text()).toContain("Nothing matched");
    expect(wrapper.findAll(".tile")).toHaveLength(0);
  });

  it("keeps current hits visible while a sort change refetches", async () => {
    const relevance = deferred<SearchPage>();
    const popular = deferred<SearchPage>();
    search.mockImplementationOnce(() => relevance.promise).mockImplementationOnce(() => popular.promise);

    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "dune");
    relevance.resolve(page([hit({ ol_work_key: "/works/OL1W", title: "Dune" })]));
    await flushPromises();

    const popularChip = wrapper.findAll("button.chip").find((chip) => chip.text() === "Popular");
    expect(popularChip).toBeTruthy();
    await popularChip!.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("Dune");

    popular.resolve(
      page([
        hit({ ol_work_key: "/works/OL1W", title: "Dune" }),
        hit({ ol_work_key: "/works/OL3W", title: "Dune Messiah" }),
      ]),
    );
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");
    expect(wrapper.text()).toContain("Dune Messiah");
  });

  it("says still searching after a couple of seconds", async () => {
    vi.useFakeTimers();
    const first = deferred<SearchPage>();
    search.mockImplementation(() => first.promise);
    const { wrapper } = await mountDiscover("/discover?q=circe");
    expect(wrapper.text()).not.toContain("Still searching");
    await vi.advanceTimersByTimeAsync(2100);
    await nextTick();
    expect(wrapper.text()).toContain("Still searching the library");
    expect(wrapper.text()).toContain("2s so far");
    first.resolve(page([hit()]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).not.toContain("Still searching");
    vi.useRealTimers();
  });

  it("ignores a stale slower response after a newer search", async () => {
    const dune = deferred<SearchPage>();
    const circe = deferred<SearchPage>();
    search.mockImplementationOnce(() => dune.promise).mockImplementationOnce(() => circe.promise);

    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "dune");
    await submitQuery(wrapper, "circe");

    circe.resolve(page([hit({ ol_work_key: "/works/OL2W", title: "Circe" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");

    dune.resolve(page([hit({ ol_work_key: "/works/OL1W", title: "Dune" })]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).not.toContain("Dune");
  });
});
