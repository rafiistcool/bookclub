import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { COVER_RETRY_MS } from "../covers";
import BookCover from "./BookCover.vue";

const circe = {
  id: 4,
  ol_work_key: "/works/OL1W",
  title: "Circe",
  cover_url: "/media/abc/l",
  cover_thumb_url: "/media/abc/s",
};

describe("BookCover", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the same-origin thumbnail on a tile, lazily", () => {
    const wrapper = mount(BookCover, { props: { book: circe, size: "tile" } });
    const img = wrapper.get("img");
    expect(img.attributes("src")).toBe("/media/abc/s");
    expect(img.attributes("loading")).toBe("lazy");
    expect(img.attributes("alt")).toBe("Cover of Circe");
  });

  it("shows the large picture full width", () => {
    const wrapper = mount(BookCover, { props: { book: circe } });
    expect(wrapper.get("img").attributes("src")).toBe("/media/abc/l");
  });

  it("shows the cloth and monogram when the book has no cover", () => {
    const wrapper = mount(BookCover, {
      props: { book: { ...circe, cover_url: null, cover_thumb_url: null } },
    });
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.get(".monogram").text()).toBe("C");
  });

  it("never renders a cover from another host", () => {
    const wrapper = mount(BookCover, {
      props: {
        book: {
          ...circe,
          cover_url: "https://covers.openlibrary.org/b/id/8739376-L.jpg",
          cover_thumb_url: "//books.google.com/x.jpg",
        },
      },
    });
    expect(wrapper.find("img").exists()).toBe(false);
  });

  it("retries a failed cover once after a short pause, then tries the other size", async () => {
    vi.useFakeTimers();
    const wrapper = mount(BookCover, { props: { book: circe, size: "md" } });
    await wrapper.get("img").trigger("error");
    expect(wrapper.find("img").exists()).toBe(false);
    vi.advanceTimersByTime(COVER_RETRY_MS);
    await nextTick();
    expect(wrapper.get("img").attributes("src")).toBe("/media/abc/s");
    await wrapper.get("img").trigger("error");
    expect(wrapper.get("img").attributes("src")).toBe("/media/abc/l");
    await wrapper.get("img").trigger("error");
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.get(".monogram").text()).toBe("C");
  });

  it("starts over when the book gets another cover", async () => {
    vi.useFakeTimers();
    const wrapper = mount(BookCover, { props: { book: circe, size: "md" } });
    await wrapper.get("img").trigger("error");
    await wrapper.setProps({ book: { ...circe, cover_thumb_url: "/media/def/s" } });
    expect(wrapper.get("img").attributes("src")).toBe("/media/def/s");
  });

  it("loads the first visible covers eagerly", () => {
    const wrapper = mount(BookCover, { props: { book: circe, eager: true } });
    expect(wrapper.get("img").attributes("loading")).toBe("eager");
  });
});
