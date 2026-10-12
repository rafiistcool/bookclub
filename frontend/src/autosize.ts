import type { ObjectDirective } from "vue";

// Phones have no resize handle, so a fixed-height textarea hides everything
// past its first few lines. Browsers with field-sizing grow it in CSS.
const native = typeof CSS !== "undefined" && CSS.supports?.("field-sizing", "content");

function fit(el: HTMLTextAreaElement) {
  if (native) return;
  el.style.height = "auto";
  if (el.scrollHeight > 0) el.style.height = `${el.scrollHeight + el.offsetHeight - el.clientHeight}px`;
}

const listeners = new WeakMap<HTMLTextAreaElement, () => void>();

/** Grow a textarea with its content: `<textarea v-autosize />`. */
export const vAutosize: ObjectDirective<HTMLTextAreaElement> = {
  mounted(el) {
    const onInput = () => fit(el);
    listeners.set(el, onInput);
    el.addEventListener("input", onInput);
    fit(el);
  },
  // Also covers drafts that are filled or cleared from code, e.g. after posting.
  updated: fit,
  unmounted(el) {
    const onInput = listeners.get(el);
    if (onInput) el.removeEventListener("input", onInput);
  },
};
