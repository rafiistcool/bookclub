import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { post, user } from "../test/fixtures";
import type { DiaryEntry as Entry } from "../types";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import DiaryEntry from "./DiaryEntry.vue";

const CLUB = { slug: "readers-ab12", name: "Readers" };

function mountEntry(entry: Entry) {
  return mount(DiaryEntry, {
    props: { entry, myProgress: null, myStatus: null },
    global: { stubs: { Avatar: true, SpoilerControl: true } },
  });
}

describe("DiaryEntry time", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    useSession().user = user({
      timezone: "Europe/Berlin",
      clubs: [{ ...CLUB, role: "member", timezone: "America/New_York", member_count: 2 }],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows a diary entry relative to now in the reader's zone, the exact time on hover", () => {
    // 2 Sep 00:30 in Berlin, 1 Sep 18:30 in New York.
    vi.setSystemTime(new Date("2026-09-01T22:30:00Z"));
    const wrapper = mountEntry(post({ created_at: "2026-09-01T20:00:00Z", created_label: "1 Sep" }));
    const time = wrapper.get(".entry-time");
    expect(time.element.tagName).toBe("TIME");
    expect(time.classes()).toEqual(expect.arrayContaining(["finer", "subtle"]));
    expect(time.text()).toBe("yesterday");
    expect(time.attributes("datetime")).toBe("2026-09-01T20:00:00Z");
    expect(time.attributes("title")).toBe("Tue, 1 Sept 2026, 22:00 CEST");
    wrapper.unmount();
  });

  it("counts a club post's days in the reader's zone, with the club's time in the title, in German too", async () => {
    // 23:00 in Berlin (the reader's zone), 17:00 in the club's New York.
    vi.setSystemTime(new Date("2026-09-01T21:00:00Z"));
    useLocale().locale = "de";
    const wrapper = mountEntry(post({ club: CLUB, created_at: "2026-09-01T20:00:00Z" }));
    expect(wrapper.get(".entry-time").text()).toBe("vor 1 Std.");
    expect(wrapper.get(".entry-time").attributes("title")).toBe("Di., 1. Sept. 2026, 16:00 GMT-4");
    // A tap shows the exact time in place, a second one the short label again.
    await wrapper.get(".entry-time").trigger("click");
    expect(wrapper.get(".entry-time").text()).toBe("Di., 1. Sept. 2026, 16:00 GMT-4");
    await wrapper.get(".entry-time").trigger("click");

    // The shared clock moves the label along without a new render from outside:
    // past midnight in Berlin it is yesterday, though still the 1st in New York.
    vi.setSystemTime(new Date("2026-09-01T22:30:00Z"));
    vi.advanceTimersByTime(60_000);
    await nextTick();
    expect(wrapper.get(".entry-time").text()).toBe("gestern");
    wrapper.unmount();
  });

  it("falls back to the server's label without a readable time", () => {
    const wrapper = mountEntry(post({ created_at: "", created_label: "1 Sep" }));
    expect(wrapper.get(".entry-time").element.tagName).toBe("SPAN");
    expect(wrapper.get(".entry-time").text()).toBe("1 Sep");
    wrapper.unmount();
  });
});

describe("DiaryEntry actions", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useSession().user = user();
  });

  it("keeps one line: reactions on the left, Reply and “…” on the right", () => {
    const wrapper = mountEntry(post({ mine: true, body: "Mine", reactions: [{ emoji: "❤️", count: 2, mine: false, users: ["a", "b"] }] }));
    const footer = wrapper.get(".entry-actions");
    expect(footer.findAll(":scope > *").map((part) => part.classes()[0])).toEqual(["reaction-row", "entry-links"]);
    expect(wrapper.findAll(".entry-links .text-btn").map((b) => b.text())).toEqual(["Reply"]);
    const more = wrapper.get(".entry-links .entry-more");
    expect(more.attributes("aria-label")).toBe("More for this post");
    expect(more.attributes("aria-haspopup")).toBe("dialog");
    // Without reactions the row keeps its shape: Reply stays on the right.
    const bare = mountEntry(post({ mine: true, body: "Mine" }));
    expect(bare.get(".entry-actions").findAll(":scope > *").map((part) => part.classes()[0])).toEqual([
      "reaction-row",
      "entry-links",
    ]);
    wrapper.unmount();
    bare.unmount();
  });

  it("opens the reaction picker on a line of its own, never under Reply and “…”", async () => {
    const wrapper = mountEntry(post({ mine: true, body: "Mine" }));
    await wrapper.get(".chip.reaction.add").trigger("click");
    const footer = wrapper.get(".entry-actions");
    // Its own flex item of the wrapping bar, after "+" in the DOM and before the links.
    expect(footer.findAll(":scope > *").map((part) => part.classes()[0])).toEqual([
      "reaction-row",
      "reaction-picker-line",
      "entry-links",
    ]);
    expect(wrapper.find(".reaction-row .reaction-picker").exists()).toBe(false);
    expect(wrapper.findAll(".reaction-picker-line .reaction-pick")).toHaveLength(6);
    wrapper.unmount();
  });

  it("edits and deletes one's own post from the “…” sheet, deleting only on a second tap", async () => {
    const wrapper = mount(DiaryEntry, {
      props: { entry: post({ mine: true, body: "Mine" }), myProgress: null, myStatus: null },
      global: { stubs: { Avatar: true, SpoilerControl: true } },
      attachTo: document.body,
    });
    await wrapper.get(".entry-more").trigger("click");
    expect(wrapper.get(".sheet").attributes("aria-label")).toBe("Your post");
    expect(wrapper.findAll(".sheet .sheet-row").map((row) => row.text())).toEqual(["Edit", "Delete"]);
    const remove = wrapper.get(".sheet .post-delete");
    await remove.trigger("click");
    expect(wrapper.emitted("delete")).toBeUndefined();
    expect(remove.text()).toBe("Really delete?");
    expect(remove.classes()).toContain("danger");
    await remove.trigger("click");
    expect(wrapper.emitted("delete")).toHaveLength(1);
    expect(wrapper.find(".sheet").exists()).toBe(false);

    await wrapper.get(".entry-more").trigger("click");
    await wrapper.get(".sheet .post-edit").trigger("click");
    await nextTick();
    expect(wrapper.find(".sheet").exists()).toBe(false);
    const field = wrapper.get(".entry-edit textarea");
    expect((field.element as HTMLTextAreaElement).value).toBe("Mine");
    expect(document.activeElement).toBe(field.element);

    // The form goes with its button: cancelling or saving gives the focus to "…".
    await wrapper.findAll(".entry-edit button").find((b) => b.text() === "Cancel")!.trigger("click");
    await nextTick();
    expect(wrapper.find(".entry-edit").exists()).toBe(false);
    expect(document.activeElement).toBe(wrapper.get(".entry-more").element);
    await wrapper.get(".entry-more").trigger("click");
    await wrapper.get(".sheet .post-edit").trigger("click");
    await nextTick();
    await wrapper.get(".entry-edit textarea").setValue("Mine, changed");
    await wrapper.get(".entry-edit").trigger("submit");
    await nextTick();
    expect(wrapper.emitted("edit")?.[0]?.[0]).toBe("Mine, changed");
    expect(document.activeElement).toBe(wrapper.get(".entry-more").element);
    wrapper.unmount();
  });

  it("offers no “…” on someone else's post without moderation rights", () => {
    const wrapper = mountEntry(post({ author: "max", body: "Hi", club: CLUB }));
    expect(wrapper.find(".entry-more").exists()).toBe(false);
    wrapper.unmount();
  });

  it("names the writer's flag and where the reader is over the blur", () => {
    const wrapper = mount(DiaryEntry, {
      props: { entry: post({ body: "Secret", spoiler_upto: 70 }), myProgress: 10, myStatus: "currently_reading" },
      global: { stubs: { Avatar: true, SpoilerControl: true } },
    });
    expect(wrapper.find(".shield-btn .kicker").exists()).toBe(false);
    expect(wrapper.get(".shield-btn .shield-label").text()).toBe("Spoiler up to 70% · you're at 10%");
    expect(wrapper.get(".shield-btn").text()).toContain("Tap to read");
    // The blur hides it from the eye only: a screen reader does not read it either.
    expect(wrapper.get(".entry-text").attributes("aria-hidden")).toBe("true");
    wrapper.unmount();
  });
});

describe("DiaryEntry spoiler fold", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useSession().user = user();
  });

  function mountAt(entry: Entry, myProgress: number | null, myStatus: Entry["status_at"]) {
    return mount(DiaryEntry, {
      props: { entry, myProgress, myStatus },
      global: { stubs: { Avatar: true, SpoilerControl: true } },
      attachTo: document.body,
    });
  }

  const ahead = post({ author: "jonas", body: "The twist!", status_at: "currently_reading", progress_at: 35 });

  it("folds a post written further on into one quiet line, and shows it on a tap", async () => {
    const wrapper = mountAt(ahead, null, null);
    expect(wrapper.text()).not.toContain("The twist!");
    expect(wrapper.find(".entry-actions").exists()).toBe(false);
    const fold = wrapper.get(".entry-folded");
    expect(fold.get(".folded-line").text()).toBe("jonas · at 35% · you're at 0%");
    expect(fold.get(".folded-hint").text()).toBe("Tap to read");
    expect(fold.attributes("aria-expanded")).toBe("false");
    await fold.trigger("click");
    expect(wrapper.text()).toContain("The twist!");
    expect(wrapper.find(".entry-folded").exists()).toBe(false);
    // The focus moves to the post, not to the page.
    expect(document.activeElement).toBe(wrapper.get("article").element);
    wrapper.unmount();
  });

  it("leaves it open for a reader as far or further, one's own, and after finishing", () => {
    for (const wrapper of [
      mountAt(ahead, 35, "currently_reading"),
      mountAt({ ...ahead, mine: true }, 0, null),
      mountAt(ahead, null, "finished"),
    ]) {
      expect(wrapper.find(".entry-folded").exists()).toBe(false);
      expect(wrapper.text()).toContain("The twist!");
      wrapper.unmount();
    }
  });
});
