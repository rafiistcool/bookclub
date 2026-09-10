import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { Favorite } from "../types";

const friendShelf = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    friendShelf: (...args: unknown[]) => friendShelf(...args),
  },
}));

import MemberShelfPage from "./MemberShelfPage.vue";

function favorite(position: number, title: string): Favorite {
  return {
    position,
    book: {
      id: position,
      ol_work_key: `/works/OL${position}W`,
      title,
      authors: "An Author",
      cover_id: position,
      year: 2018,
      cover_url: null,
    },
  };
}

async function mountMember(favorites: Favorite[]) {
  friendShelf.mockResolvedValue({
    user: {
      id: 2,
      username: "grace",
      theme: "paper",
      color_mode: "system",
      locale: "en",
      avatar_url: null,
    },
    items: [],
    favorites,
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/club/:username", component: MemberShelfPage }],
  });
  await router.push("/club/grace");
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
    expect(wrapper.get(".portrait").findAll("a")).toHaveLength(1);
    expect(wrapper.get(".portrait a").attributes("aria-label")).toBe("1. Circe");
  });
});
