import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

const createCustomBook = vi.fn();
const push = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    body: unknown;
    constructor(message: string, status: number, _detail?: unknown, body?: unknown) {
      super(message);
      this.status = status;
      this.body = body;
    }
  },
  api: {
    createCustomBook: (...args: unknown[]) => createCustomBook(...args),
  },
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push }),
}));

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import AddBookSheet from "./AddBookSheet.vue";

describe("AddBookSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    createCustomBook.mockReset();
    push.mockReset();
    createCustomBook.mockResolvedValue({ id: 31, title: "Kitchen Zine 2" });
  });

  it("submits a numeric year from the number input without crashing", async () => {
    const wrapper = mount(AddBookSheet, {
      props: { initialTitle: "Kitchen Zine 2" },
      global: { plugins: [createPinia(), i18n] },
    });
    await wrapper.get('input[type="text"]').setValue("Kitchen Zine 2");
    const author = wrapper.findAll("input")[1];
    await author.setValue("Ada");
    await wrapper.get('input[type="number"]').setValue("2024");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(createCustomBook).toHaveBeenCalledWith({
      title: "Kitchen Zine 2",
      authors: "Ada",
      year: 2024,
      description: "",
    });
    expect(push).toHaveBeenCalledWith("/book/31");
  });

  it("sends an ISBN and offers the book the catalog already has for it", async () => {
    createCustomBook.mockRejectedValue(
      new ApiError("That ISBN is already in the catalog", 409, "That ISBN is already in the catalog", {
        detail: "That ISBN is already in the catalog",
        work_id: 12,
        edition_id: 40,
      }),
    );
    const wrapper = mount(AddBookSheet, {
      props: { initialTitle: "Circe" },
      global: { plugins: [createPinia(), i18n] },
    });
    const isbn = wrapper.findAll('input[type="text"]')[2];
    await isbn.setValue(" 9780316556347 ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(createCustomBook.mock.calls[0][0]).toMatchObject({ title: "Circe", isbn: "9780316556347" });
    expect(wrapper.text()).toContain("That ISBN is already in the catalog");
    await wrapper.findAll("button").find((button) => button.text() === "Open that book")!.trigger("click");
    await flushPromises();
    expect(push).toHaveBeenCalledWith("/book/12");
  });

  it("puts an initial ISBN into the ISBN field, not the title", async () => {
    const wrapper = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488" },
      global: { plugins: [createPinia(), i18n] },
    });
    const fields = wrapper.findAll<HTMLInputElement>('input[type="text"]');
    expect(fields[0].element.value).toBe("");
    expect(fields[2].element.value).toBe("9780316769488");
    expect(wrapper.text()).toContain("The ISBN you looked up. The catalog checks it first.");
    await fields[0].setValue("Kitchen Zine 2");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(createCustomBook.mock.calls[0][0]).toMatchObject({ title: "Kitchen Zine 2", isbn: "9780316769488" });
    expect(push).toHaveBeenCalledWith("/book/31");
  });

  it("shows the usual hint once the initial ISBN is changed", async () => {
    const wrapper = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488" },
      global: { plugins: [createPinia(), i18n] },
    });
    await wrapper.findAll('input[type="text"]')[2].setValue("3518368818");
    expect(wrapper.text()).toContain("Optional. The catalog checks it first.");
    expect(wrapper.text()).not.toContain("The ISBN you looked up.");
  });

  it("with stay, emits the new book and closes instead of opening it", async () => {
    const wrapper = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488", stay: true },
      global: { plugins: [createPinia(), i18n] },
    });
    await wrapper.get('input[type="text"]').setValue("Kitchen Zine 2");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(wrapper.emitted("added")).toEqual([[{ id: 31, title: "Kitchen Zine 2" }, "9780316769488"]]);
    expect(wrapper.emitted("close")).toHaveLength(1);
    expect(push).not.toHaveBeenCalled();
  });

  it("with stay, emits the ISBN as saved: a changed one as typed, a cleared one as null", async () => {
    const changed = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488", stay: true },
      global: { plugins: [createPinia(), i18n] },
    });
    await changed.get('input[type="text"]').setValue("Kitchen Zine 2");
    await changed.findAll('input[type="text"]')[2].setValue(" 0-306-40615-2 ");
    await changed.get("form").trigger("submit");
    await flushPromises();
    expect(changed.emitted("added")?.[0][1]).toBe("0-306-40615-2");

    const cleared = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488", stay: true },
      global: { plugins: [createPinia(), i18n] },
    });
    await cleared.get('input[type="text"]').setValue("Kitchen Zine 2");
    await cleared.findAll('input[type="text"]')[2].setValue("");
    await cleared.get("form").trigger("submit");
    await flushPromises();
    expect(cleared.emitted("added")?.[0][1]).toBeNull();
  });

  it("with stay, keeps the form and its values when the save fails", async () => {
    createCustomBook.mockRejectedValue(
      new ApiError("Could not look that ISBN up right now. Try again.", 502, "Could not look that ISBN up right now. Try again."),
    );
    const wrapper = mount(AddBookSheet, {
      props: { initialIsbn: "9780316769488", stay: true },
      global: { plugins: [createPinia(), i18n] },
    });
    await wrapper.get('input[type="text"]').setValue("Kitchen Zine 2");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(wrapper.text()).toContain("Could not look that ISBN up right now. Try again.");
    expect(wrapper.emitted("added")).toBeUndefined();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(wrapper.findAll<HTMLInputElement>('input[type="text"]')[2].element.value).toBe("9780316769488");
  });
});
