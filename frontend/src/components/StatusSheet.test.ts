import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import StatusSheet from "./StatusSheet.vue";

describe("StatusSheet", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    document.body.classList.remove("sheet-open");
  });

  it("marks the current status and makes the likely next step primary", () => {
    const wrapper = mount(StatusSheet, {
      props: { title: "Move to…", current: "currently_reading" },
      attachTo: document.body,
    });
    const options = wrapper.findAll(".status-option");
    expect(options).toHaveLength(4);
    const current = options.find((o) => o.attributes("aria-current") === "true");
    expect(current?.text()).toContain("Reading");
    expect(current?.text()).toContain("Current");
    expect(current?.classes()).not.toContain("btn-primary");
    const primary = options.find((o) => o.classes().includes("btn-primary"));
    expect(primary?.text()).toContain("Finished");
    wrapper.unmount();
  });

  it("defaults to Want to read as primary for a new book", () => {
    const wrapper = mount(StatusSheet, { props: { title: "Add" }, attachTo: document.body });
    const primary = wrapper.findAll(".status-option").find((o) => o.classes().includes("btn-primary"));
    expect(primary?.text()).toContain("Want to read");
    wrapper.unmount();
  });

  it("emits the chosen status", async () => {
    const wrapper = mount(StatusSheet, { props: { title: "Add" }, attachTo: document.body });
    const options = wrapper.findAll(".status-option");
    await options[3].trigger("click");
    expect(wrapper.emitted("pick")?.[0]).toEqual(["did_not_finish"]);
    wrapper.unmount();
  });
});
