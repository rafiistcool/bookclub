import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { edition } from "../test/fixtures";

const addEdition = vi.fn();

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api/client")>()),
  api: {
    addEdition: (...args: unknown[]) => addEdition(...args),
  },
}));

import { ApiError } from "../api/client";
import AddEditionForm from "./AddEditionForm.vue";

function mountForm(props: { workId: number; initialIsbn?: string }) {
  return mount(AddEditionForm, { props, global: { plugins: [createPinia()] } });
}

describe("AddEditionForm", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    addEdition.mockReset();
  });

  it("starts with an empty ISBN and asks for at least one field", async () => {
    const wrapper = mountForm({ workId: 3 });
    expect(wrapper.get<HTMLInputElement>("input[name='isbn']").element.value).toBe("");
    expect(wrapper.text()).toContain("On the back cover or the copyright page. Optional.");
    await wrapper.get("form").trigger("submit");
    expect(wrapper.text()).toContain("Fill in at least one field.");
    expect(addEdition).not.toHaveBeenCalled();
  });

  it("fills the ISBN from initialIsbn and sends it", async () => {
    const added = edition({ id: 77, work_id: 3, isbn13: "9780316769488" });
    addEdition.mockResolvedValue(added);
    const wrapper = mountForm({ workId: 3, initialIsbn: "9780316769488" });
    expect(wrapper.get<HTMLInputElement>("input[name='isbn']").element.value).toBe("9780316769488");
    expect(wrapper.get<HTMLInputElement>("input[name='title']").element.value).toBe("");
    expect(wrapper.text()).toContain("The ISBN you looked up.");
    await wrapper.get("select[name='format']").setValue("paperback");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(addEdition).toHaveBeenCalledWith(3, {
      isbn: "9780316769488",
      format: "paperback",
      publisher: null,
      publication_year: null,
      pages: null,
      language: null,
      title: null,
    });
    expect(wrapper.emitted("added")).toEqual([[added]]);
  });

  it("keeps the 409 choice when the ISBN is an edition of this book already", async () => {
    addEdition.mockRejectedValue(
      new ApiError("That ISBN is already in the catalog", 409, "That ISBN is already in the catalog", {
        detail: "That ISBN is already in the catalog",
        work_id: 3,
        edition_id: 40,
      }),
    );
    const wrapper = mountForm({ workId: 3, initialIsbn: "9780316769488" });
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(wrapper.text()).toContain("That ISBN is already in the catalog");
    // The choice names the ISBN the 409 was for, also when the field changes afterwards.
    await wrapper.get("input[name='isbn']").setValue("0-306-40615-2");
    const choose = wrapper.findAll("button").find((button) => button.text() === "Choose that edition");
    await choose!.trigger("click");
    expect(wrapper.emitted("existing")).toEqual([[40, "9780316769488"]]);
  });
});
