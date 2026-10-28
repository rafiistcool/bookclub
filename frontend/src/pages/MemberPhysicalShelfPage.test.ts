import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { shelfItem } from "../test/fixtures";
import type { ShelfItem } from "../types";

const friendShelf = vi.fn();
const physicalShelfLayout = vi.fn();
const savePhysicalShelfLayout = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    friendShelf: (...args: unknown[]) => friendShelf(...args),
    physicalShelfLayout: (...args: unknown[]) => physicalShelfLayout(...args),
    savePhysicalShelfLayout: (...args: unknown[]) => savePhysicalShelfLayout(...args),
  },
}));

import MemberPhysicalShelfPage from "./MemberPhysicalShelfPage.vue";

function item(id: number, title: string): ShelfItem {
  return shelfItem({
    id, status: "finished", position: id, shelf_position: id,
    updated_at: "2026-10-08T10:00:00Z", rating: 4, take: "Stays with you.",
    book: { id: id + 100, title, authors: "An Author", year: 2020, ol_work_key: `/works/OL${id}W` },
  });
}

async function mountMember(layout: unknown, items = [item(1, "Circe"), item(2, "Piranesi")]) {
  friendShelf.mockResolvedValue({
    timezone: "UTC",
    user: { id: 2, username: "grace", avatar_url: null },
    items, favorites: [],
  });
  physicalShelfLayout.mockResolvedValue(layout);
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/u/:username/shelf", component: MemberPhysicalShelfPage }],
  });
  await router.push("/u/grace/shelf");
  await router.isReady();
  const wrapper = mount(MemberPhysicalShelfPage, { global: { plugins: [createPinia(), router] } });
  await flushPromises();
  return wrapper;
}

describe("another member's bookshelf", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    friendShelf.mockReset();
    physicalShelfLayout.mockReset();
    savePhysicalShelfLayout.mockReset();
  });

  it("shows their arrangement without anything to move, remove or restyle", async () => {
    const wrapper = await mountMember({
      rows: [[2, "decor:globe-1"], [1]], material: "black", lights: true,
      spines: { "1": { display: "cover" } },
      decorations: { "decor:globe-1": { kind: "globe", color: "#33485f" } },
    });
    expect(friendShelf).toHaveBeenCalledWith("grace");
    expect(physicalShelfLayout).toHaveBeenCalledWith("grace");
    const rows = wrapper.findAll(".physical-shelf-row");
    expect(rows.map((row) => row.findAll(".shelf-object").map((object) => object.attributes("data-id")))).toEqual([["2", "decor:globe-1"], ["1"]]);
    expect(wrapper.get(".physical-shelf").attributes("data-material")).toBe("black");
    expect(wrapper.find(".shelf-lights").exists()).toBe(true);
    expect(wrapper.find(".remove-shelf-row").exists()).toBe(false);
    expect(wrapper.find(".material-choice").exists()).toBe(false);
    expect(wrapper.get(".shelf-decoration").attributes("disabled")).toBeDefined();

    await wrapper.get('.shelf-object[data-id="1"]').trigger("click");
    expect(wrapper.get(".spine-sheet-book").text()).toContain("Circe");
    expect(wrapper.text()).toContain("Stays with you.");
    expect(wrapper.get(".open-book").attributes("href")).toBe("/book/101");
    expect(wrapper.find(".edit-spine").exists()).toBe(false);
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
  });

  it("arranges the books itself when the member never set up a bookshelf", async () => {
    const wrapper = await mountMember({ rows: [[], [], []], material: "oak", spines: {}, decorations: {} });
    expect(wrapper.findAll(".book-spine")).toHaveLength(2);
    expect(savePhysicalShelfLayout).not.toHaveBeenCalled();
  });

  it("says so when there is nothing on the shelf", async () => {
    const wrapper = await mountMember({ rows: [[], [], []], material: "oak", spines: {}, decorations: {} }, []);
    expect(wrapper.find(".physical-shelf").exists()).toBe(false);
    expect(wrapper.text()).toContain("grace has no books");
  });
});
