import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clubPick, shelfItem, user } from "../test/fixtures";
import type { ClubSummary, ShelfItem } from "../types";

const patchShelf = vi.fn();
const clubPickCall = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.name = "ApiError";
      this.status = status;
    }
  },
  api: {
    patchShelf: (...args: unknown[]) => patchShelf(...args),
    clubPick: (...args: unknown[]) => clubPickCall(...args),
  },
}));

import { defineComponent, ref } from "vue";
import { ApiError } from "../api/client";
import { useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import FinishedMoment from "./FinishedMoment.vue";
import ProgressEditor from "./ProgressEditor.vue";

const CLUB: ClubSummary = { slug: "readers", name: "Readers", role: "member", timezone: "UTC", member_count: 4 };

function finished(overrides: Parameters<typeof shelfItem>[0] = {}): ShelfItem {
  return shelfItem({ id: 9, status: "finished", book: { id: 4, title: "Circe", authors: "Madeline Miller" }, ...overrides });
}

function open(item: ShelfItem = finished(), clubSlug: string | null = null, clubs: ClubSummary[] = []) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ username: "ada", clubs });
  const finish = useFinish();
  const answer = finish.celebrate({ item, clubSlug });
  const wrapper = mount(FinishedMoment, { global: { plugins: [pinia] }, attachTo: document.body });
  return { wrapper, finish, answer };
}

function stars(wrapper: ReturnType<typeof open>["wrapper"]) {
  return wrapper.findAll(".moment-star");
}

function setReducedMotion(reduce: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: reduce && query.includes("reduce"),
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
}

describe("FinishedMoment", () => {
  const originalMatchMedia = window.matchMedia;

  beforeEach(() => {
    patchShelf.mockReset();
    clubPickCall.mockReset();
  });

  afterEach(() => {
    window.matchMedia = originalMatchMedia;
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("is a dialog titled “Finished!” with the book, five stars and no club call without a club", () => {
    const { wrapper } = open();
    const dialog = wrapper.get("[role=dialog]");
    expect(dialog.attributes("aria-label")).toBe("Finished!");
    expect(wrapper.get(".sheet-head h2").text()).toBe("Finished!");
    expect(wrapper.get(".moment-title").text()).toBe("Circe");
    expect(stars(wrapper)).toHaveLength(5);
    expect(stars(wrapper).map((star) => star.attributes("aria-pressed"))).toEqual(Array(5).fill("false"));
    expect(clubPickCall).not.toHaveBeenCalled();
    expect(wrapper.find(".moment-club").exists()).toBe(false);
    wrapper.unmount();
  });

  it("fills the stars one after another and ticks under the finger", async () => {
    vi.useFakeTimers();
    const vibrate = vi.fn();
    Object.defineProperty(navigator, "vibrate", { value: vibrate, configurable: true });
    const { wrapper } = open();
    await stars(wrapper)[3].trigger("click");
    expect(vibrate).toHaveBeenCalledWith(6);
    expect(stars(wrapper)[3].attributes("aria-pressed")).toBe("true");
    // The first star at once, the rest 30 ms apart.
    expect(wrapper.findAll(".moment-star.lit")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(30);
    expect(wrapper.findAll(".moment-star.lit")).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(60);
    expect(wrapper.findAll(".moment-star.lit")).toHaveLength(4);
    // Tapping the chosen star again keeps the rating.
    await stars(wrapper)[3].trigger("click");
    expect(stars(wrapper)[3].attributes("aria-pressed")).toBe("true");
    wrapper.unmount();
    Reflect.deleteProperty(navigator, "vibrate");
  });

  it("fills the stars at once under reduced motion", async () => {
    setReducedMotion(true);
    const { wrapper } = open();
    await stars(wrapper)[4].trigger("click");
    expect(wrapper.findAll(".moment-star.lit")).toHaveLength(5);
    wrapper.unmount();
  });

  it("saves the stars, the line and “only me” in one PATCH on “Done”", async () => {
    const item = finished();
    const { wrapper, finish, answer } = open(item);
    await stars(wrapper)[4].trigger("click");
    expect(wrapper.find(".moment-take").exists()).toBe(false);
    await wrapper.get(".moment-add-take").trigger("click");
    await wrapper.get(".moment-take textarea").setValue("Gods, but human.");
    expect(wrapper.get(".moment-take textarea").attributes("maxlength")).toBe("140");
    await wrapper.get(".moment-only-me input").setValue(true);
    const saved = { ...item, rating: 5, take: "Gods, but human.", rating_visibility: "only_me" as const };
    patchShelf.mockResolvedValue(saved);
    await wrapper.findAll("button").find((node) => node.text() === "Save")!.trigger("click");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledTimes(1);
    expect(patchShelf).toHaveBeenCalledWith(9, { rating: 5, take: "Gods, but human.", rating_visibility: "only_me" });
    await expect(answer).resolves.toStrictEqual(saved);
    expect(finish.item).toBeNull();
    expect(finish.lastSaved).toStrictEqual(saved);
    wrapper.unmount();
  });

  it("sends only the rating when nothing else was set", async () => {
    const { wrapper } = open();
    await stars(wrapper)[2].trigger("click");
    patchShelf.mockResolvedValue(finished({ rating: 3 }));
    await wrapper.findAll("button").find((node) => node.text() === "Save")!.trigger("click");
    await flushPromises();
    expect(patchShelf).toHaveBeenCalledWith(9, { rating: 3 });
    wrapper.unmount();
  });

  it("closes without a request on “Done” with nothing set, and on “Skip”", async () => {
    const first = open();
    await first.wrapper.findAll("button").find((node) => node.text() === "Save")!.trigger("click");
    await expect(first.answer).resolves.toBeNull();
    first.wrapper.unmount();

    const second = open();
    await stars(second.wrapper)[1].trigger("click");
    await second.wrapper.findAll("button").find((node) => node.text() === "Skip")!.trigger("click");
    await expect(second.answer).resolves.toBeNull();
    expect(second.finish.item).toBeNull();
    expect(patchShelf).not.toHaveBeenCalled();
    second.wrapper.unmount();
  });

  it("stays open with the error when the save fails", async () => {
    const { wrapper, finish } = open();
    await stars(wrapper)[0].trigger("click");
    patchShelf.mockRejectedValue(new ApiError("Book not on your shelf", 404, "Book not on your shelf"));
    await wrapper.findAll("button").find((node) => node.text() === "Save")!.trigger("click");
    await flushPromises();
    expect(wrapper.get("[role=alert]").text()).toBe("Book not on your shelf");
    expect(finish.item).not.toBeNull();
    wrapper.unmount();
  });

  it("says how many in the club are done when the book is its current pick", async () => {
    clubPickCall.mockResolvedValue({ pick: clubPick({ book: finished().book, finished: ["ada", "grace"] }), timezone: "UTC" });
    const { wrapper } = open(finished(), "readers", [CLUB]);
    await flushPromises();
    expect(clubPickCall).toHaveBeenCalledWith("readers");
    expect(wrapper.get(".moment-club").text()).toBe("2 of 4 in the club are done");
    wrapper.unmount();
  });

  it("counts the reader when the pick's list lags, and says when everyone is done", async () => {
    clubPickCall.mockResolvedValue({ pick: clubPick({ book: finished().book, finished: ["grace", "lin", "mo"] }), timezone: "UTC" });
    const { wrapper } = open(finished(), "readers", [CLUB]);
    await flushPromises();
    expect(wrapper.get(".moment-club").text()).toBe("All 4 in the club are done");
    wrapper.unmount();
  });

  it("drops the club line silently when the pick is another book, the call fails or the club is not the reader's", async () => {
    clubPickCall.mockResolvedValueOnce({ pick: clubPick({ book: { ...finished().book, id: 99 } }), timezone: "UTC" });
    const other = open(finished(), "readers", [CLUB]);
    await flushPromises();
    expect(other.wrapper.find(".moment-club").exists()).toBe(false);
    other.wrapper.unmount();

    clubPickCall.mockRejectedValueOnce(new Error("offline"));
    const failed = open(finished(), "readers", [CLUB]);
    await flushPromises();
    expect(failed.wrapper.find(".moment-club").exists()).toBe(false);
    failed.wrapper.unmount();

    const stranger = open(finished(), "elsewhere", [CLUB]);
    await flushPromises();
    expect(clubPickCall).toHaveBeenCalledTimes(2);
    expect(stranger.wrapper.find(".moment-club").exists()).toBe(false);
    stranger.wrapper.unmount();
  });

  it("does not close a newer moment when its own save returns late", async () => {
    const { wrapper, finish } = open();
    let saved!: (item: ShelfItem) => void;
    patchShelf.mockReturnValue(new Promise((resolve) => (saved = resolve)));
    await stars(wrapper)[3]!.trigger("click");
    await wrapper.get(".moment-actions .btn-primary").trigger("click");
    // Another finish opens the next moment while this "Save" is still saving.
    const next = finish.celebrate({ item: finished({ id: 10, book: { id: 5, title: "Dune" } }) });
    saved(finished({ rating: 4 }));
    await flushPromises();
    expect(finish.item?.id).toBe(10);
    expect(finish.lastSaved?.id).toBe(9);
    finish.close(null, finish.serial);
    await expect(next).resolves.toBeNull();
  });

  it("gives focus back near the progress row the finish removed", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ username: "ada" });
    // A page whose reading row leaves the list with the finished book, like Home's.
    const Page = defineComponent({
      components: { ProgressEditor },
      setup: () => ({ reading: ref(true) }),
      template: `<main><ul class="reading-list">
        <li v-if="reading"><ProgressEditor :value="95" label="Progress" book-title="Circe" :shelf-id="9" @finished="reading = false" /></li>
        <li><a href="/book/2">Dune</a></li>
      </ul></main>`,
    });
    const page = mount(Page, { global: { plugins: [pinia] }, attachTo: document.body });
    const trigger = page.get<HTMLButtonElement>(".progress-trigger");
    trigger.element.focus();
    await trigger.trigger("click");
    await page.get('.sheet button[aria-label="Forward 5 percent"]').trigger("click");
    patchShelf.mockResolvedValue(finished());
    await page.findAll(".sheet button").find((button) => button.text() === "Mark as read")!.trigger("click");
    await flushPromises();
    expect(page.findAll("li")).toHaveLength(1);

    // App.vue mounts the moment once its chunk is here; Skip closes it and App unmounts it.
    const finish = useFinish();
    const moment = mount(FinishedMoment, { global: { plugins: [pinia] }, attachTo: document.body });
    await moment.get(".moment-actions .btn-ghost").trigger("click");
    expect(finish.item).toBeNull();
    moment.unmount();
    expect(document.activeElement).toBe(page.get(".reading-list").element);
    page.unmount();
  });
});
