import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

const createCustomBook = vi.fn();
const push = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    createCustomBook: (...args: unknown[]) => createCustomBook(...args),
  },
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push }),
}));

import AddBookSheet from "./AddBookSheet.vue";

describe("AddBookSheet", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    createCustomBook.mockReset();
    push.mockReset();
    createCustomBook.mockResolvedValue({
      ol_work_key: "/works/BCdeadbeef01",
      title: "Kitchen Zine 2",
    });
  });

  it("submits a numeric year from the number input without crashing", async () => {
    const wrapper = mount(AddBookSheet, {
      props: { initialTitle: "Kitchen Zine 2" },
      global: { plugins: [createPinia()] },
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
    expect(push).toHaveBeenCalledWith("/book/BCdeadbeef01");
  });
});
