import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { defineComponent } from "vue";
import { i18n } from "../i18n";
import DiaryPage from "./DiaryPage.vue";

const DiaryFeed = defineComponent({
  name: "DiaryFeed",
  props: { headingHidden: Boolean, heading: String, mine: Boolean },
  template: "<div class='feed' />",
});

describe("DiaryPage", () => {
  it("keeps the feed's heading for screen readers only, under the page title", () => {
    const wrapper = mount(DiaryPage, { global: { plugins: [i18n], stubs: { DiaryFeed } } });
    const feed = wrapper.getComponent(DiaryFeed);
    expect(feed.props("headingHidden")).toBe(true);
    expect(feed.props("mine")).toBe(true);
  });
});
