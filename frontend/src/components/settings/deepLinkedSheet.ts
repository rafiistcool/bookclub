import { onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";

/**
 * Which of a page's sheets is open. A link can open one with `?sheet=<name>`
 * (the old `/settings#…` anchors redirect there, see settingsRoutes.ts); the
 * page drops the query once it has read it, so a reload or Back does not open
 * the sheet again.
 */
export function useDeepLinkedSheet<T extends string>(names: readonly T[]) {
  const route = useRoute();
  const router = useRouter();
  const asked = route.query.sheet;
  const sheet = ref<T | null>(
    typeof asked === "string" && (names as readonly string[]).includes(asked) ? (asked as T) : null,
  );

  onMounted(() => {
    if (route.query.sheet === undefined) return;
    const { sheet: _dropped, ...query } = route.query;
    void router.replace({ path: route.path, query, hash: route.hash });
  });

  return {
    sheet,
    open(name: T) {
      sheet.value = name;
    },
    close() {
      sheet.value = null;
    },
  };
}
