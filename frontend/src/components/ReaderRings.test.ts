import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { i18n } from "../i18n";
import ReaderRings, { type RingReader } from "./ReaderRings.vue";

const reader = (username: string, status: string, progress: number | null = null): RingReader => ({
  username,
  avatar_url: null,
  status,
  progress,
});

function mountRings(props: { readers: RingReader[]; memberCount?: number; size?: "sm" | "md"; max?: number }) {
  return mount(ReaderRings, { props, global: { plugins: [i18n] } });
}

const labels = (wrapper: ReturnType<typeof mountRings>) =>
  wrapper.findAll("li .visually-hidden").map((row) => row.text());

describe("ReaderRings", () => {
  it("draws each reader's progress as a ring and says it in words", () => {
    const wrapper = mountRings({
      readers: [reader("mara", "currently_reading", 35), reader("jonas", "finished"), reader("lena", "want_to_read")],
    });
    expect(wrapper.get("ul").attributes("aria-label")).toBe("Where everyone is");
    expect(labels(wrapper)).toEqual(["mara: 35%", "jonas: finished", "lena: To read"]);
    const fills = wrapper.findAll(".ring-fill");
    // Lena has not started: only the track.
    expect(fills).toHaveLength(2);
    expect(fills[0]!.attributes("pathLength")).toBe("100");
    expect(fills[0]!.attributes("style")).toContain("stroke-dashoffset: 65");
    expect(fills[1]!.attributes("style")).toContain("stroke-dashoffset: 0");
    // Only the finished reader gets the check.
    expect(wrapper.findAll(".ring-check")).toHaveLength(1);
    expect(wrapper.findAll(".ring-item")[1]!.find(".ring-check").exists()).toBe(true);
    // md puts the names below, hidden from screen readers (the label says it).
    expect(wrapper.findAll(".ring-name").map((name) => name.text())).toEqual(["mara", "jonas", "lena"]);
  });

  it("counts the readers beyond max and the members who have not started", () => {
    const wrapper = mountRings({
      readers: [reader("a", "currently_reading", 10), reader("b", "currently_reading", 20), reader("c", "finished")],
      memberCount: 6,
      max: 2,
    });
    expect(wrapper.findAll(".ring-item:not(.ring-overflow):not(.ring-waiting)")).toHaveLength(2);
    expect(wrapper.get(".ring-overflow .ring-count").text()).toBe("+1");
    expect(wrapper.get(".ring-waiting .ring-count").text()).toBe("3");
    expect(labels(wrapper)).toEqual(["a: 10%", "b: 20%", "1 more", "3 not started yet"]);
    expect(wrapper.get(".ring-waiting .ring-name").text()).toBe("not yet");
  });

  it("leaves the names out in sm and clamps odd progress", () => {
    const wrapper = mountRings({ readers: [reader("mara", "currently_reading", 140)], size: "sm", memberCount: 1 });
    expect(wrapper.find(".ring-name").exists()).toBe(false);
    expect(wrapper.find(".ring-waiting").exists()).toBe(false);
    expect(labels(wrapper)).toEqual(["mara: 100%"]);
    expect(wrapper.get("ul").classes()).toContain("size-sm");
  });
});
