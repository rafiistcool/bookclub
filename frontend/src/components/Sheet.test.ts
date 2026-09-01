import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import Sheet from "./Sheet.vue";

function mountSheet() {
  return mount(Sheet, {
    props: { title: "Move to…", subtitle: "Circe" },
    slots: {
      default: '<button id="a">A</button><button id="b">B</button>',
      foot: '<button id="save">Save</button>',
    },
    attachTo: document.body,
  });
}

describe("Sheet", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    document.body.classList.remove("sheet-open");
    delete document.body.dataset.sheets;
  });

  it("renders as a modal dialog with handle, title and subtitle", () => {
    const wrapper = mountSheet();
    const dialog = wrapper.get('[role="dialog"]');
    expect(dialog.attributes("aria-modal")).toBe("true");
    expect(dialog.attributes("aria-label")).toBe("Move to…");
    expect(wrapper.find(".sheet-handle").exists()).toBe(true);
    expect(wrapper.text()).toContain("Circe");
    wrapper.unmount();
  });

  it("locks body scroll while open and releases it on unmount", () => {
    const wrapper = mountSheet();
    expect(document.body.classList.contains("sheet-open")).toBe(true);
    wrapper.unmount();
    expect(document.body.classList.contains("sheet-open")).toBe(false);
  });

  it("keeps the lock while a nested sheet is still open", () => {
    const first = mountSheet();
    const second = mountSheet();
    second.unmount();
    expect(document.body.classList.contains("sheet-open")).toBe(true);
    first.unmount();
    expect(document.body.classList.contains("sheet-open")).toBe(false);
  });

  it("emits close on Escape, backdrop tap and the close button", async () => {
    const wrapper = mountSheet();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.get(".sheet-backdrop").trigger("click");
    await wrapper.get(".sheet-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(3);
    wrapper.unmount();
  });

  it("moves initial focus past the close button and traps Tab at both ends", async () => {
    const wrapper = mountSheet();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const close = document.querySelector(".sheet-close") as HTMLElement;
    const firstContent = document.getElementById("a") as HTMLElement;
    const last = document.getElementById("save") as HTMLElement;
    expect(document.activeElement).toBe(firstContent);

    // Tab from the last focusable wraps to the first (the close button).
    last.focus();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(close);

    // Shift+Tab from the first wraps to the last.
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(last);

    // Focus outside the sheet is pulled back in.
    (document.body as HTMLElement).focus();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(last);
    wrapper.unmount();
  });

  it("prefers a data-autofocus element", async () => {
    const wrapper = mount(Sheet, {
      props: { title: "T" },
      slots: { default: '<button id="x">X</button><input id="y" data-autofocus />' },
      attachTo: document.body,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.activeElement?.id).toBe("y");
    wrapper.unmount();
  });
});
