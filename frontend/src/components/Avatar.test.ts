import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import Avatar from "./Avatar.vue";

describe("Avatar", () => {
  it("shows the username initial when there is no picture", () => {
    const wrapper = mount(Avatar, { props: { username: "ada" } });
    expect(wrapper.text()).toBe("A");
    expect(wrapper.find("img").exists()).toBe(false);
  });

  it("renders the picture when a src is set", () => {
    const wrapper = mount(Avatar, {
      props: { username: "ada", src: "/api/members/ada/avatar?v=1" },
    });
    expect(wrapper.find("img").attributes("src")).toBe("/api/members/ada/avatar?v=1");
    expect(wrapper.find("img").attributes("loading")).toBe("lazy");
    expect(wrapper.text()).toBe("");
  });

  it("falls back to the initial when the picture fails to load", async () => {
    const wrapper = mount(Avatar, {
      props: { username: "ada", src: "/api/members/ada/avatar?v=1" },
    });
    await wrapper.find("img").trigger("error");
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.text()).toBe("A");
  });

  it("shows the picture again after src changes", async () => {
    const wrapper = mount(Avatar, {
      props: { username: "ada", src: "/broken" },
    });
    await wrapper.find("img").trigger("error");
    await wrapper.setProps({ src: "/api/members/ada/avatar?v=2" });
    expect(wrapper.find("img").attributes("src")).toBe("/api/members/ada/avatar?v=2");
    expect(wrapper.text()).toBe("");
  });
});
