import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { favorite as favoriteRow, shelfItem } from "../test/fixtures";
import type { Favorite, ShelfItem } from "../types";

const friendShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status = 404;
  },
  api: {
    friendShelf: (...args: unknown[]) => friendShelf(...args),
  },
}));

import { ApiError } from "../api/client";
import MemberShelfPage from "./MemberShelfPage.vue";

function favorite(position: number, title: string): Favorite {
  return favoriteRow(position, { title, authors: "An Author", year: 2018 });
}

async function mountMember(favorites: Favorite[], items: ShelfItem[] = []) {
  friendShelf.mockResolvedValue({
    timezone: "UTC",
    user: { id: 2, username: "grace", avatar_url: null },
    items,
    favorites,
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/u/:username", component: MemberShelfPage }],
  });
  await router.push("/u/grace");
  await router.isReady();
  const wrapper = mount(MemberShelfPage, {
    global: {
      plugins: [createPinia(), router],
    },
  });
  await flushPromises();
  return wrapper;
}

describe("MemberShelfPage favourites portrait", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    friendShelf.mockReset();
  });

  it("hides the portrait when the member has no favourites", async () => {
    const wrapper = await mountMember([]);
    expect(wrapper.find(".portrait").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("No favourites yet");
  });

  it("shows the member's favourite covers under the avatar", async () => {
    const wrapper = await mountMember([favorite(1, "Circe")]);
    const portrait = wrapper.get("ul.portrait");
    expect(portrait.findAll("li a")).toHaveLength(1);
    expect(portrait.get("li a").attributes("aria-label")).toBe("1. Circe");
    expect(portrait.get("li a").attributes("role")).toBeUndefined();
  });

  it("names the empty shelf once, in the heading", async () => {
    const wrapper = await mountMember([]);
    expect(wrapper.get("h1").text()).toBe("Shelf of grace");
    expect(wrapper.find(".lede").exists()).toBe(false);
    const empty = wrapper.get(".empty");
    expect(empty.get("h3").text()).toBe("Nothing on this shelf yet");
    expect(empty.find("p").exists()).toBe(false);
  });

  it("stacks each status filter's count under its label", async () => {
    const items = [
      shelfItem({ id: 1, status: "want_to_read", book: { id: 11, title: "Circe" } }),
      shelfItem({ id: 2, status: "did_not_finish", book: { id: 12, title: "Ulysses" } }),
    ];
    const wrapper = await mountMember([], items);
    const filter = wrapper.get(".shelf-filter");
    expect(filter.classes()).toContain("stacked");
    const segments = filter.findAll("button").map((button) => [button.get(".seg-label").text(), button.get(".seg-count").text()]);
    expect(segments).toEqual([["All", "2"], ["To read", "1"], ["Reading", "0"], ["Read", "0"], ["DNF", "1"]]);
  });

  it("shows the same not-found answer for a stranger as for an unknown name", async () => {
    friendShelf.mockRejectedValue(new ApiError("Nobody with that name shares a club with you", 404, "No member with that name"));
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/u/:username", component: MemberShelfPage }],
    });
    await router.push("/u/stranger");
    await router.isReady();
    const wrapper = mount(MemberShelfPage, { global: { plugins: [createPinia(), router] } });
    await flushPromises();
    expect(friendShelf).toHaveBeenCalledWith("stranger");
    expect(wrapper.text()).toContain("Nobody with that name shares a club with you");
    expect(wrapper.find(".portrait").exists()).toBe(false);
  });
});
