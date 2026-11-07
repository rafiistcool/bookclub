import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { bookDetail } from "../test/fixtures";
import type { BookDetail } from "../types";
import BookActionsSheet from "./BookActionsSheet.vue";

function mountSheet(book: Partial<BookDetail> = {}, props: { withdrawn?: boolean } = {}) {
  return mount(BookActionsSheet, {
    props: {
      book: bookDetail({ id: 4, ...book }),
      title: "Circe",
      admin: false,
      coverPrompted: false,
      withdrawn: props.withdrawn ?? false,
      refreshing: false,
    },
  });
}

describe("BookActionsSheet", () => {
  it("offers Choose cover only while the book is on the reader's shelf", async () => {
    const browsing = mountSheet();
    expect(browsing.find(".choose-cover").exists()).toBe(false);
    // A book without a cover and off the shelf has no cover group at all.
    expect(browsing.findAll("h3").map((node) => node.text())).toEqual(["About this book"]);

    const shelved = mountSheet({ shelf_id: 30, on_shelf: "want_to_read" });
    const row = shelved.get(".choose-cover");
    expect(row.text()).toBe("Choose cover");
    expect(shelved.findAll("h3").map((node) => node.text())).toEqual(["About this book", "Cover"]);
    await row.trigger("click");
    expect(shelved.emitted("act")).toEqual([["choose-cover"]]);
  });

  it("offers it for the reader's own cover too, but not for a withdrawn book", () => {
    const own = mountSheet({ shelf_id: 30, on_shelf: "finished", personal_cover: true, cover_url: "/media/own/l", cover_thumb_url: "/media/own/s", cover_public_id: "own" });
    expect(own.find(".choose-cover").exists()).toBe(true);
    const withdrawn = mountSheet({ shelf_id: 30, on_shelf: "finished", custom: true, source: "user" }, { withdrawn: true });
    expect(withdrawn.find(".choose-cover").exists()).toBe(false);
  });
});
