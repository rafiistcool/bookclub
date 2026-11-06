import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter, type Router } from "vue-router";
import type { PixelImage, Quad } from "../coverScan/geometry";
import { bookCover, cornerError, posedCover, renderScene, solid } from "../coverScan/scenes";

// jsdom has no canvas, so the parts that decode, read and encode pictures are
// stood in for; the detector and the geometry are the real ones, run on a
// synthetic photo.
const loadForScan = vi.fn();
const readPixels = vi.fn();
const flatten = vi.fn();

vi.mock("../coverScan", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../coverScan")>()),
  loadForScan: (...args: unknown[]) => loadForScan(...args),
  readPixels: (...args: unknown[]) => readPixels(...args),
  flatten: (...args: unknown[]) => flatten(...args),
}));

import { i18n } from "../i18n";
import { useCoverScan } from "../stores/coverScan";
import { useToast } from "../stores/toast";
import CoverScanner from "./CoverScanner.vue";
import { rotateQuad } from "../coverScan/geometry";

/** A 1000 × 1500 photo; the detector sees a 200 × 300 copy of it. */
const bitmap = { close: vi.fn() };
const photo = new File(["photo"], "photo.jpg", { type: "image/jpeg" });
const flat = new File(["flat"], "cover.jpg", { type: "image/jpeg" });
/** insetQuad(1000, 1500): 8 % in from each side. */
const INSET: Quad = [
  { x: 80, y: 120 },
  { x: 920, y: 120 },
  { x: 920, y: 1380 },
  { x: 80, y: 1380 },
];

/** The detector's copy: a printed cover turned by 8° on a dark table, and its true corners there. */
const COVER = posedCover(200, 300, { width: 120, height: 180, rotate: 8 });
const COVER_PHOTO: PixelImage = renderScene({
  width: 200,
  height: 300,
  quad: COVER,
  cover: bookCover([228, 218, 196], [30, 28, 40], [40, 120, 130]),
  background: solid([55, 50, 48]),
  noise: 4,
});
/** Nothing to find: an even grey. */
const EVEN_PHOTO: PixelImage = { width: 200, height: 300, data: new Uint8ClampedArray(200 * 300 * 4).fill(128) };

let wrapper: VueWrapper | null = null;

async function openScan(file = photo, router?: Router) {
  wrapper = mount(CoverScanner, { attachTo: document.body, global: { plugins: router ? [router] : [] } });
  const answer = useCoverScan().request(file);
  await flushPromises();
  // Wrapped: an async function returning the promise would wait for it.
  return { answer };
}

async function ready() {
  await vi.waitFor(() => expect(wrapper!.get(".scan-apply").attributes("aria-disabled")).toBe("false"));
}

function tab(shiftKey = false): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(event);
  return event;
}

function pages(): Router {
  const page = { render: () => null };
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/a", component: page },
      { path: "/b", component: page },
    ],
  });
}

/** jsdom has no PointerEvent: a mouse event with a pointer id stands in. */
async function pointer(target: { element: Element }, type: string, clientX = 0, clientY = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX, clientY });
  Object.defineProperty(event, "pointerId", { value: 1 });
  target.element.dispatchEvent(event);
  await flushPromises();
}

function corner(name: string) {
  return wrapper!.get(`[data-corner="${name}"]`);
}

beforeEach(() => {
  setActivePinia(createPinia());
  i18n.global.locale.value = "en";
  // jsdom has no canvas; the drawing finds no context and skips.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  bitmap.close.mockReset();
  loadForScan.mockReset().mockResolvedValue({ bitmap, width: 1000, height: 1500 });
  readPixels.mockReset().mockReturnValue({ data: EVEN_PHOTO, scale: 0.2 });
  flatten.mockReset().mockResolvedValue(flat);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
});

describe("CoverScanner", () => {
  it("shows nothing until a photo is requested", () => {
    wrapper = mount(CoverScanner, { attachTo: document.body });
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it("opens as a modal dialog over the page, and resolves null on cancel", async () => {
    const trigger = document.createElement("button");
    document.body.appendChild(trigger);
    trigger.focus();
    const { answer } = await openScan();
    await ready();
    const dialog = wrapper!.get('[role="dialog"]');
    expect(dialog.attributes("aria-modal")).toBe("true");
    expect(dialog.attributes("aria-label")).toBe("Straighten the cover");
    expect(document.activeElement).toBe(dialog.element);
    expect(document.body.style.overflow).toBe("hidden");
    expect(loadForScan).toHaveBeenCalledWith(photo);

    await wrapper!.get(".scan-cancel").trigger("click");
    await expect(answer).resolves.toBeNull();
    await flushPromises();
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
    expect(document.body.style.overflow).toBe("");
    expect(document.activeElement).toBe(trigger);
    expect(bitmap.close).toHaveBeenCalled();
    expect(flatten).not.toHaveBeenCalled();
    trigger.remove();
  });

  it("resolves null on Escape", async () => {
    const { answer } = await openScan();
    await ready();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await expect(answer).resolves.toBeNull();
  });

  it("hands the photo back as it is with “Whole photo”", async () => {
    const { answer } = await openScan();
    await ready();
    await wrapper!.get(".scan-whole").trigger("click");
    await expect(answer).resolves.toBe(photo);
    expect(flatten).not.toHaveBeenCalled();
  });

  it("starts inset from the edges when the detector finds no cover", async () => {
    const { answer } = await openScan();
    await ready();
    expect(readPixels).toHaveBeenCalledWith(bitmap, 1000, 1500, expect.any(Number));
    expect(wrapper!.get(".scan-status").text()).toBe("Drag the corners to the edges of the cover");
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, INSET);
  });

  it("starts at the detected corners, scaled to the photo", async () => {
    readPixels.mockReturnValue({ data: COVER_PHOTO, scale: 0.2 });
    const { answer } = await openScan();
    await ready();
    expect(wrapper!.get(".scan-status").text()).toBe("Corners found");
    // Applied mid-glide, the corners land where they were heading.
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    const used = flatten.mock.calls[0][3] as Quad;
    const truth = COVER.map((p) => ({ x: p.x * 5, y: p.y * 5 })) as Quad;
    // Within 2 % of the photo's diagonal (1803 px).
    expect(cornerError(used, truth)).toBeLessThan(36);
  });

  it("names each corner and moves it with the arrow keys, faster with Shift", async () => {
    const { answer } = await openScan();
    await ready();
    expect(["tl", "tr", "br", "bl"].map((name) => corner(name).attributes("aria-label"))).toEqual([
      "Top-left corner",
      "Top-right corner",
      "Bottom-right corner",
      "Bottom-left corner",
    ]);
    await corner("tl").trigger("keydown", { key: "ArrowRight" });
    await corner("tl").trigger("keydown", { key: "ArrowDown", shiftKey: true });
    await corner("br").trigger("keydown", { key: "ArrowLeft" });
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, [
      { x: 81, y: 130 },
      { x: 920, y: 120 },
      { x: 919, y: 1380 },
      { x: 80, y: 1380 },
    ]);
  });

  it("drags a corner, and keeps the last valid point when a move would fold the quad", async () => {
    const { answer } = await openScan();
    await ready();
    const tl = corner("tl");
    await pointer(tl, "pointerdown", 80, 120);
    await pointer(tl, "pointermove", 100, 130);
    expect(wrapper!.find(".scan-loupe").exists()).toBe(true);
    // Past the top-right corner: the quad would cross itself, so the move is refused.
    await pointer(tl, "pointermove", 990, 130);
    await pointer(tl, "pointerup");
    expect(wrapper!.find(".scan-loupe").exists()).toBe(false);
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, [{ x: 100, y: 130 }, INSET[1], INSET[2], INSET[3]]);
  });

  it("drags an edge by its midpoint, moving both of its corners", async () => {
    const { answer } = await openScan();
    await ready();
    const top = wrapper!.findAll(".scan-edge")[0]!;
    await pointer(top, "pointerdown", 500, 120);
    await pointer(top, "pointermove", 500, 150);
    await pointer(top, "pointerup");
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, [{ x: 80, y: 150 }, { x: 920, y: 150 }, INSET[2], INSET[3]]);
  });

  it("refuses a key move onto or past the neighbouring corner", async () => {
    const { answer } = await openScan();
    await ready();
    // The top-left corner pushed right, ten px a press, into the top-right one and beyond.
    for (let i = 0; i < 200; i++) await corner("tl").trigger("keydown", { key: "ArrowRight", shiftKey: true });
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    const [tl] = flatten.mock.calls[0][3] as Quad;
    expect(tl).toEqual({ x: 910, y: 120 });
  });

  it("shows a busy state while flattening and stays open when it fails", async () => {
    let fail!: (error: Error) => void;
    flatten.mockReturnValue(new Promise((_resolve, reject) => (fail = reject)));
    const { answer } = await openScan();
    await ready();
    const apply = wrapper!.get(".scan-apply");
    (apply.element as HTMLElement).focus();
    await apply.trigger("click");
    expect(apply.text()).toBe("Straightening…");
    expect(wrapper!.get(".scan-status").text()).toBe("Straightening…");
    // aria-disabled, not disabled: the focus stays on the button.
    for (const name of [".scan-apply", ".scan-cancel", ".scan-whole", ".scan-rotate"]) {
      expect(wrapper!.get(name).attributes("aria-disabled")).toBe("true");
      expect(wrapper!.get(name).attributes("disabled")).toBeUndefined();
    }
    await vi.waitFor(() => expect(flatten).toHaveBeenCalled());
    expect(document.activeElement).toBe(apply.element);
    // Busy, the other buttons do nothing.
    await wrapper!.get(".scan-cancel").trigger("click");
    await wrapper!.get(".scan-whole").trigger("click");
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(true);
    fail(new Error("no canvas"));
    await flushPromises();
    expect(useToast().items.at(-1)?.message).toBe("Could not straighten the photo");
    // Said inside the dialog too, where a screen reader still hears it.
    expect(wrapper!.get('[role="dialog"] [role="alert"]').text()).toBe("Could not straighten the photo");
    expect(document.activeElement).toBe(apply.element);
    expect(apply.text()).toBe("Apply");
    expect(apply.attributes("aria-disabled")).toBe("false");
    await wrapper!.get(".scan-whole").trigger("click");
    await expect(answer).resolves.toBe(photo);
  });

  it("lets the busy state paint before the warp starts", async () => {
    const { answer } = await openScan();
    await ready();
    await wrapper!.get(".scan-apply").trigger("click");
    // The click's own task only sets the state; the flattening waits for a frame.
    expect(flatten).not.toHaveBeenCalled();
    expect(wrapper!.get(".scan-apply").text()).toBe("Straightening…");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledTimes(1);
  });

  it("keeps Shift+Tab from the dialog itself inside it", async () => {
    const behind = document.createElement("button");
    document.body.appendChild(behind);
    await openScan();
    await ready();
    const dialog = wrapper!.get('[role="dialog"]').element;
    expect(document.activeElement).toBe(dialog);
    const back = tab(true);
    expect(back.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(wrapper!.get(".scan-apply").element);
    // From the first control, Shift+Tab wraps to the last; from the last, Tab to the first.
    (wrapper!.get('[data-corner="tl"]').element as HTMLElement).focus();
    tab(true);
    expect(document.activeElement).toBe(wrapper!.get(".scan-apply").element);
    tab();
    expect(document.activeElement).toBe(wrapper!.get('[data-corner="tl"]').element);
    (dialog as HTMLElement).focus();
    expect(tab().defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(wrapper!.get('[data-corner="tl"]').element);
    behind.remove();
  });

  it("gives focus to the label when the input that opened it is disabled by then", async () => {
    // CoverPhotoButton: a file input in a label, disabled while it uploads.
    const label = document.createElement("label");
    const input = document.createElement("input");
    input.type = "file";
    label.appendChild(input);
    document.body.appendChild(label);
    input.focus();
    const { answer } = await openScan();
    input.disabled = true;
    await ready();
    await wrapper!.get(".scan-cancel").trigger("click");
    await expect(answer).resolves.toBeNull();
    await flushPromises();
    expect(document.activeElement).toBe(label);
    expect(label.getAttribute("tabindex")).toBe("-1");
    // Only while it holds the focus.
    label.blur();
    expect(label.hasAttribute("tabindex")).toBe(false);
    label.remove();
  });

  it("gives focus to the nearest focusable ancestor when the opener is gone", async () => {
    const section = document.createElement("section");
    section.tabIndex = -1;
    const trigger = document.createElement("button");
    section.appendChild(trigger);
    document.body.appendChild(section);
    trigger.focus();
    await openScan();
    trigger.remove();
    await ready();
    await wrapper!.get(".scan-cancel").trigger("click");
    await flushPromises();
    expect(document.activeElement).toBe(section);
    section.remove();
  });

  it("falls back to the main region rather than the body", async () => {
    const main = document.createElement("main");
    const trigger = document.createElement("button");
    document.body.append(main, trigger);
    trigger.focus();
    await openScan();
    trigger.remove();
    await ready();
    await wrapper!.get(".scan-cancel").trigger("click");
    await flushPromises();
    expect(document.activeElement).toBe(main);
    main.remove();
  });

  it("darkens the browser chrome while open and gives the colour back", async () => {
    const meta = document.createElement("meta");
    meta.name = "theme-color";
    meta.content = "#f7f4ec";
    document.head.appendChild(meta);
    const style = document.createElement("style");
    style.textContent = ".scanner { --bg: #16130f; }";
    document.head.appendChild(style);
    await openScan();
    await ready();
    expect(meta.content).toBe("#16130f");
    await wrapper!.get(".scan-cancel").trigger("click");
    await flushPromises();
    expect(meta.content).toBe("#f7f4ec");
    meta.remove();
    style.remove();
  });

  it("turns the cover's top an edge on, clockwise, and flattens it that way", async () => {
    const { answer } = await openScan();
    await ready();
    const rotate = wrapper!.get(".scan-rotate");
    expect(rotate.attributes("aria-label")).toBe("Rotate clockwise");
    expect(wrapper!.get(".scan-top").attributes("data-side")).toBe("top");
    await rotate.trigger("click");
    // Clockwise: what is on the left of the photo becomes the cover's top.
    expect(wrapper!.get(".scan-top").attributes("data-side")).toBe("left");
    expect(wrapper!.get(".scan-status").text()).toBe("The top of the cover is now the left edge");
    // The corners keep their places and names on screen.
    expect(corner("tl").attributes("aria-label")).toBe("Top-left corner");
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, rotateQuad(INSET, 1));
  });

  it("comes round to the photo's own top after four turns", async () => {
    const { answer } = await openScan();
    await ready();
    const sides: string[] = [];
    for (let i = 0; i < 4; i++) {
      await wrapper!.get(".scan-rotate").trigger("click");
      sides.push(wrapper!.get(".scan-top").attributes("data-side")!);
    }
    expect(sides).toEqual(["left", "bottom", "right", "top"]);
    await wrapper!.get(".scan-apply").trigger("click");
    await expect(answer).resolves.toBe(flat);
    expect(flatten).toHaveBeenCalledWith(bitmap, 1000, 1500, INSET);
  });

  it("cancels on Back and keeps the page", async () => {
    const router = pages();
    await router.push("/a");
    await router.push("/b");
    const { answer } = await openScan(photo, router);
    await ready();
    router.back();
    await expect(answer).resolves.toBeNull();
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/b");
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
    // Closed, the guard is gone: navigation works again.
    await router.push("/a");
    expect(router.currentRoute.value.path).toBe("/a");
  });

  it("cancels on any other navigation too, and stays on the page", async () => {
    const router = pages();
    await router.push("/a");
    const { answer } = await openScan(photo, router);
    await ready();
    const failure = await router.push("/b");
    expect(failure).toBeTruthy();
    await expect(answer).resolves.toBeNull();
    expect(router.currentRoute.value.path).toBe("/a");
  });

  it("hands a photo it cannot open back as it is", async () => {
    loadForScan.mockRejectedValue(new Error("undecodable"));
    const { answer } = await openScan();
    await expect(answer).resolves.toBe(photo);
    await flushPromises();
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
  });

  it("takes one photo at a time", async () => {
    const { answer } = await openScan();
    await ready();
    await expect(useCoverScan().request(new File(["2"], "two.jpg"))).resolves.toBeNull();
    expect(loadForScan).toHaveBeenCalledTimes(1);
    await wrapper!.get(".scan-cancel").trigger("click");
    await expect(answer).resolves.toBeNull();
  });

  it("speaks German", async () => {
    i18n.global.locale.value = "de";
    await openScan();
    await ready();
    expect(wrapper!.get(".scan-status").text()).toBe("Zieh die Ecken an den Rand des Covers");
    expect(wrapper!.get(".scan-whole").text()).toBe("Ganzes Bild");
    expect(wrapper!.get(".scan-apply").text()).toBe("Übernehmen");
    expect(corner("tl").attributes("aria-label")).toBe("Ecke oben links");
    expect(wrapper!.get(".scan-top").text()).toBe("oben");
    expect(wrapper!.get(".scan-rotate").attributes("aria-label")).toBe("Im Uhrzeigersinn drehen");
  });
});
