import { onUnmounted, ref, type Ref } from "vue";

/** Reactive `matchMedia`. Used to switch layouts that CSS alone cannot, like
    mounting the drag-and-drop board only on wide viewports. */
export function useMediaQuery(query: string): Ref<boolean> {
  const list = window.matchMedia(query);
  const matches = ref(list.matches);
  const update = (event: MediaQueryListEvent) => {
    matches.value = event.matches;
  };
  list.addEventListener("change", update);
  onUnmounted(() => list.removeEventListener("change", update));
  return matches;
}

export const DESKTOP = "(min-width: 1024px)";
