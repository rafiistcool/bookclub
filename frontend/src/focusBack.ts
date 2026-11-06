/**
 * Giving focus back after an overlay closes (the cover scanner, the finished
 * moment). The element that opened it may be gone by then: a disabled file
 * input, a progress row the finish removed. So the caller notes the opener's
 * ancestry while it is still on the page, and focus goes to the nearest of
 * them still there that takes it, at worst the page's main region, never
 * silently to the body.
 */

function tryFocus(el: HTMLElement): boolean {
  el.focus({ preventScroll: true });
  return document.activeElement === el;
}

/** Focus an element that is not focusable on its own, until it loses focus again. */
export function focusTemporarily(el: HTMLElement): boolean {
  if (el.hasAttribute("tabindex")) return tryFocus(el);
  el.setAttribute("tabindex", "-1");
  if (tryFocus(el)) {
    el.addEventListener("blur", () => el.removeAttribute("tabindex"), { once: true });
    return true;
  }
  el.removeAttribute("tabindex");
  return false;
}

/** The element and its ancestors below the body, nearest first. Note it while the element is on the page. */
export function ancestry(el: Element | null): HTMLElement[] {
  const chain: HTMLElement[] = [];
  for (let node = el; node && node !== document.body; node = node.parentElement) {
    if (node instanceof HTMLElement) chain.push(node);
  }
  return chain;
}

/**
 * Focus the first element of `chain` that is still on the page and takes
 * focus (a label is focused for the moment), else the page's main region.
 * With `nearest`, the first one still on the page is focused for the moment
 * even when it takes no focus of its own (the list a removed row was in), so
 * the next Tab continues from there.
 */
export function giveFocusBack(chain: HTMLElement[], { nearest = false } = {}) {
  for (const el of chain) {
    if (!el.isConnected || el === document.body || el === document.documentElement) continue;
    if (tryFocus(el)) return;
    if ((nearest || el instanceof HTMLLabelElement) && focusTemporarily(el)) return;
  }
  const main = document.querySelector<HTMLElement>("main") ?? document.querySelector<HTMLElement>("#app");
  if (main) focusTemporarily(main);
}

/** Whether focus has fallen off the page: nothing, the body, or an element no longer there. */
export function focusLost(): boolean {
  const active = document.activeElement;
  return !active || active === document.body || !active.isConnected;
}
