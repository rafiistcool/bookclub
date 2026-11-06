import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookDetail, edition } from "../test/fixtures";

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: () => vi.fn() }),
}));

import { i18n } from "../i18n";
import EditRequestSheet from "./EditRequestSheet.vue";

function mountSheet(props: InstanceType<typeof EditRequestSheet>["$props"]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  return mount(EditRequestSheet, { props, global: { plugins: [pinia] } });
}

describe("EditRequestSheet", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });

  it("says only who checks a reader's correction, without repeating the title", () => {
    const wrapper = mountSheet({ book: bookDetail({ id: 4, title: "Circe" }) });
    const blurb = wrapper.get(".sheet p.fine.muted");
    expect(blurb.text()).toBe("An admin checks your change before everyone sees it.");
    expect(wrapper.get<HTMLInputElement>("input[name='title'], textarea[name='title']").element.value).toBe("Circe");

    const forEdition = mountSheet({ edition: edition({ id: 13, title: "Mein Heft" }) });
    expect(forEdition.get(".sheet p.fine.muted").text()).toBe("An admin checks your change before everyone sees it.");
  });

  it("names the book in an admin's edit, which applies at once", () => {
    const wrapper = mountSheet({ book: bookDetail({ id: 4, title: "Circe" }), admin: true });
    expect(wrapper.get(".sheet p.fine.muted").text()).toBe(
      "Your values for “Circe” apply at once, and no catalog source overwrites them later.",
    );
  });
});
