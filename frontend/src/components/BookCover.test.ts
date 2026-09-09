import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import BookCover from "./BookCover.vue";

describe("BookCover", () => {
  it("requests a medium tile with default=false, not a large GIF-friendly URL", () => {
    const wrapper = mount(BookCover, {
      props: { title: "Circe", coverId: 8739376, size: "tile" },
    });
    const img = wrapper.get("img");
    expect(img.attributes("src")).toBe(
      "https://covers.openlibrary.org/b/id/8739376-M.jpg?default=false",
    );
    expect(img.attributes("loading")).toBe("lazy");
  });

  it("does not render an image for null or -1 cover ids", () => {
    const missing = mount(BookCover, { props: { title: "Circe", coverId: -1 } });
    expect(missing.find("img").exists()).toBe(false);
    expect(missing.text()).toContain("C");
  });

  it("does not invent an Open Library ISBN CDN URL for Google catalog keys", () => {
    const wrapper = mount(BookCover, {
      props: { title: "Circe", workKey: "/works/ISBN9780316769488", size: "md" },
    });
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.text()).toContain("C");
  });

  it("renders an https cover URL and skips Open Library sources", () => {
    const wrapper = mount(BookCover, {
      props: {
        title: "Circe",
        imageUrl:
          "http://books.google.com/books/content?id=zyTCAlFPjgYC&printsec=frontcover",
        coverId: 8739376,
        workKey: "/works/ISBN9780316769488",
        size: "md",
      },
    });
    expect(wrapper.get("img").attributes("src")).toBe(
      "https://books.google.com/books/content?id=zyTCAlFPjgYC&printsec=frontcover",
    );
  });

  it("uses an ISBN prop for Open Library works when cover_id is missing", () => {
    const wrapper = mount(BookCover, {
      props: {
        title: "Circe",
        workKey: "/works/OL1W",
        isbn: "9780316769488",
        size: "md",
      },
    });
    expect(wrapper.get("img").attributes("src")).toBe(
      "https://covers.openlibrary.org/b/isbn/9780316769488-M.jpg?default=false",
    );
  });

  it("falls back to an edition key when cover_id is missing", () => {
    const wrapper = mount(BookCover, {
      props: { title: "Circe", coverId: null, coverEditionKey: "OL1M", size: "md" },
    });
    expect(wrapper.get("img").attributes("src")).toBe(
      "https://covers.openlibrary.org/b/olid/OL1M-M.jpg?default=false",
    );
  });

  it("loads the first visible covers eagerly", () => {
    const wrapper = mount(BookCover, {
      props: { title: "Circe", coverId: 1, eager: true },
    });
    expect(wrapper.get("img").attributes("loading")).toBe("eager");
  });
});
