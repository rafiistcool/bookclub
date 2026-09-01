import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ProgressBar from "./ProgressBar.vue";

describe("ProgressBar", () => {
  it("clamps and exposes the value for assistive tech", () => {
    const wrapper = mount(ProgressBar, { props: { value: 140, label: "Dune progress" } });
    const bar = wrapper.get('[role="progressbar"]');
    expect(bar.attributes("aria-valuenow")).toBe("100");
    expect(bar.attributes("aria-label")).toBe("Dune progress");
    expect(wrapper.text()).toContain("100%");
    expect(wrapper.get(".progress > span").attributes("style")).toContain("--value: 100%");
  });

  it("treats null as zero and can hide the number", () => {
    const wrapper = mount(ProgressBar, { props: { value: null, hideValue: true } });
    expect(wrapper.get('[role="progressbar"]').attributes("aria-valuenow")).toBe("0");
    expect(wrapper.text()).not.toContain("%");
  });
});
