import { watch, type Ref } from "vue";

/**
 * Resolves once `busy` is false. An Undo tapped while the page is still
 * saving something else waits for that save instead of being dropped.
 */
export function whenIdle(busy: Ref<boolean>): Promise<void> {
  if (!busy.value) return Promise.resolve();
  return new Promise((resolve) => {
    const stop = watch(busy, (now) => {
      if (now) return;
      stop();
      resolve();
    });
  });
}
