import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { createMemoryHistory, createRouter, type Router } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SearchHit, SearchPage } from "../types";

const search = vi.fn();
const trending = vi.fn();
const subject = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    search: (...args: unknown[]) => search(...args),
    trending: (...args: unknown[]) => trending(...args),
    subject: (...args: unknown[]) => subject(...args),
  },
}));

import DiscoverPage from "./DiscoverPage.vue";

function hit(id: string, title: string): SearchHit {
  return {
    ol_work_key: `/works/${id}`,
    title,
    authors: "Author",
    cover_id: 1,
    year: 2020,
    on_shelf: null,
    shelf_id: null,
  };
}

function pageOf(items: SearchHit[], page = 1, has_more = false): SearchPage {
  return { items, page, has_more };
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
        plugins: [router],
        stubs: { BookTile: bookTileStub },
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

describe("DiscoverPage search results", () => {
  beforeEach(() => {
    search.mockReset();
    trending.mockReset();
    subject.mockReset();
    trending.mockResolvedValue(pageOf([hit("OLT", "Trending Book")]));
    subject.mockResolvedValue(pageOf([]));
  });

  it("does not empty visible results after they land while the route catches up", async () => {
    const first = deferred<SearchPage>();
    search.mockImplementationOnce(() => first.promise);

    const { wrapper, router } = await mountDiscover();
    const titles = () => wrapper.findAll(".tile").map((n) => n.text());

    await submitQuery(wrapper, "dune");
    expect(search).toHaveBeenCalledTimes(1);
    expect(titles()).not.toContain("Dune");

    first.resolve(pageOf([hit("OL1W", "Dune")]));
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
    dune.resolve(pageOf([hit("OL1W", "Dune")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");

    await submitQuery(wrapper, "circe");
    await nextTick();
    expect(wrapper.text()).toContain("Dune");
    expect(wrapper.text()).not.toContain("Nothing matched");

    circe.resolve(pageOf([hit("OL2W", "Circe")]));
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
    dune.resolve(pageOf([hit("OL1W", "Dune")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");

    await wrapper.get("button.text-btn").trigger("click");
    await nextTick();
    expect(wrapper.text()).not.toContain("Dune");

    await submitQuery(wrapper, "circe");
    await nextTick();
    expect(wrapper.text()).not.toContain("Dune");
    expect(wrapper.text()).not.toContain("Circe");

    circe.resolve(pageOf([hit("OL2W", "Circe")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
  });

  it("still shows the empty state when a reset search returns nothing", async () => {
    search.mockResolvedValue(pageOf([]));
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
    relevance.resolve(pageOf([hit("OL1W", "Dune")]));
    await flushPromises();

    const popularChip = wrapper.findAll("button.chip").find((chip) => chip.text() === "Popular");
    expect(popularChip).toBeTruthy();
    await popularChip!.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("Dune");

    popular.resolve(pageOf([hit("OL1W", "Dune"), hit("OL3W", "Dune Messiah")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Dune");
    expect(wrapper.text()).toContain("Dune Messiah");
  });

  it("ignores a stale slower response after a newer search", async () => {
    const dune = deferred<SearchPage>();
    const circe = deferred<SearchPage>();
    search.mockImplementationOnce(() => dune.promise).mockImplementationOnce(() => circe.promise);

    const { wrapper } = await mountDiscover();
    await submitQuery(wrapper, "dune");
    await submitQuery(wrapper, "circe");

    circe.resolve(pageOf([hit("OL2W", "Circe")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");

    dune.resolve(pageOf([hit("OL1W", "Dune")]));
    await flushPromises();
    expect(wrapper.text()).toContain("Circe");
    expect(wrapper.text()).not.toContain("Dune");
  });
});
