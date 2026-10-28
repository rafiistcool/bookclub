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
});
