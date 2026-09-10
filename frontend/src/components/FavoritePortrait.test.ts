import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Favorite } from "../types";
import FavoritePortrait from "./FavoritePortrait.vue";

function favorite(overrides: Partial<Favorite["book"]> & { position?: number } = {}): Favorite {
  const { position = 1, ...book } = overrides;
  return {
    position,
    book: {
      id: book.id ?? position,
      ol_work_key: book.ol_work_key ?? `/works/OL${position}W`,
      title: book.title ?? "Circe",
      authors: book.authors ?? "Madeline Miller",
      cover_id: book.cover_id ?? 123,
      year: book.year ?? 2018,
      cover_url: book.cover_url ?? null,
    },
  };
}

describe("FavoritePortrait", () => {
  it("hides when empty and no hint is given", () => {
    const wrapper = mount(FavoritePortrait, { props: { items: [] } });
    expect(wrapper.find(".portrait").exists()).toBe(false);
    expect(wrapper.text()).toBe("");
  });

  it("shows a subtle empty hint on your own shelf", () => {
    const wrapper = mount(FavoritePortrait, {
      props: { items: [], emptyHint: "No favourites yet. Mark up to three from a book page." },
    });
    expect(wrapper.find(".portrait").exists()).toBe(false);
    expect(wrapper.text()).toContain("No favourites yet");
  });

  it("renders ordered covers with rank labels", () => {
    const wrapper = mount(FavoritePortrait, {
      props: {
        items: [
          favorite({ position: 1, title: "Circe", id: 1 }),
          favorite({ position: 2, title: "Galatea", id: 2, cover_id: 9 }),
        ],
      },
    });
    const portrait = wrapper.get("ul.portrait");
    expect(portrait.element.tagName).toBe("UL");
    expect(portrait.attributes("aria-label")).toBe("Favourite books");
    expect(portrait.findAll("li")).toHaveLength(2);
    const links = portrait.findAll("a");
    expect(links).toHaveLength(2);
    expect(links[0].attributes("href")).toBe("/book/OL1W");
    expect(links[0].attributes("aria-label")).toBe("1. Circe");
    expect(links[0].attributes("role")).toBeUndefined();
    expect(links[1].attributes("aria-label")).toBe("2. Galatea");
    expect(links[1].attributes("role")).toBeUndefined();
    expect(wrapper.findAll(".portrait-rank").map((node) => node.text())).toEqual(["1", "2"]);
    expect(wrapper.findAll("img")).toHaveLength(2);
  });
});
