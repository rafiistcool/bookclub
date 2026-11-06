import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { i18n } from "../i18n";
import BottomSheet from "./BottomSheet.vue";

function mountSheet() {
  return mount(BottomSheet, {
    props: { title: "Report" },
    slots: { default: '<textarea class="note"></textarea>' },
    global: { plugins: [i18n] },
  });
}

// jsdom has no PointerEvent; a plain event with the fields the sheet reads.
function pointer(type: string, clientY: number): Event {
  const event = new Event(type, { bubbles: true });
  Object.defineProperties(event, { clientY: { value: clientY }, pointerId: { value: 1 } });
  return event;
}

async function drag(wrapper: ReturnType<typeof mountSheet>, by: number) {
  const grip = wrapper.get(".sheet-grip").element;
  for (const [type, y] of [["pointerdown", 100], ["pointermove", 100 + by / 2], ["pointermove", 100 + by], ["pointerup", 100 + by]] as const) {
    grip.dispatchEvent(pointer(type, y));
  }
  await wrapper.vm.$nextTick();
  await wrapper.vm.$nextTick();
}

describe("BottomSheet", () => {
  it("closes on a drag down the grip, and with its close button", async () => {
    const wrapper = mountSheet();
    await drag(wrapper, 200);
    expect(wrapper.emitted("close")).toHaveLength(1);
    await wrapper.get(".sheet-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);
    wrapper.unmount();
  });

  it("keeps a sheet with something typed in it open on a drag", async () => {
    const wrapper = mountSheet();
    await wrapper.get("textarea.note").setValue("Half a sentence");
    await drag(wrapper, 300);
    expect(wrapper.emitted("close")).toBeUndefined();
    // It gave a little under the finger and springs back.
    expect(wrapper.get(".sheet").attributes("style") ?? "").not.toContain("300px");
    // The deliberate way out still works.
    await wrapper.get(".sheet-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
    wrapper.unmount();
  });
});
