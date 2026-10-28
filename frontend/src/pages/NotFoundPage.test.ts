import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { i18n } from "../i18n";
import NotFoundPage from "./NotFoundPage.vue";

describe("NotFoundPage", () => {
  it("says there is nothing here and leads Home or to Discover", async () => {
    i18n.global.locale.value = "en";
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/:pathMatch(.*)*", component: NotFoundPage }],
    });
    await router.push("/nonexistent/path");
    const wrapper = mount(NotFoundPage, { global: { plugins: [router] } });
    expect(wrapper.find("h1").text()).toBe("Page not found");
    const links = wrapper.findAll("a").map((link) => [link.attributes("href"), link.text()]);
    expect(links).toEqual([
      ["/", "Home"],
      ["/discover", "Discover"],
    ]);
  });
});
