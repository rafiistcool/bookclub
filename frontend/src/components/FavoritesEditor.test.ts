import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Favorite } from "../types";
import FavoritesEditor from "./FavoritesEditor.vue";

function favorite(position: number, title: string, id = position): Favorite {
  return {
    position,
    book: {
      id,
      ol_work_key: `/works/OL${id}W`,
      title,
      authors: "An Author",
      cover_id: id,
      year: 2018,
      cover_url: null,
    },
  };
}

describe("FavoritesEditor", () => {
  it("shows three slots and emits a compacted replace", async () => {
    const wrapper = mount(FavoritesEditor, {
      props: {
        items: [favorite(1, "Circe", 1), favorite(2, "Galatea", 2)],
      },
    });
    expect(wrapper.text()).toContain("Position 1");
    expect(wrapper.text()).toContain("Position 3");
    expect(wrapper.text()).toContain("Empty");

    await wrapper.get('[aria-label="Move down"]').trigger("click");
    expect(wrapper.emitted("replace")?.[0]).toEqual([[2, 1]]);

    await wrapper.get('[aria-label="Clear this favourite"]').trigger("click");
    expect(wrapper.emitted("replace")?.[1]).toEqual([[2]]);
  });

  it("clears the set", async () => {
    const wrapper = mount(FavoritesEditor, {
      props: { items: [favorite(1, "Circe", 1)] },
    });
    await wrapper.get("button.btn-ghost:not(.btn-sm)").trigger("click");
    expect(wrapper.emitted("replace")?.[0]).toEqual([[]]);
  });
});
