import { ref, type Ref } from "vue";
import { ApiError } from "../../api/client";
import { nextQueuePage } from "../../admin";
import { t } from "../../i18n";
import { useToast } from "../../stores/toast";

/** The open items, or the decided ones (the history). */
export type QueueView = "open" | "resolved";

type Page<T> = { items: T[]; page: number; has_more: boolean };

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

/**
 * One admin queue as a paged list (`?status=&page=`): the open items oldest
 * first or the decided ones newest first, 50 a page, with "Load more". An
 * answer that arrives after the view changed is dropped.
 */
export function useQueue<T extends { id: number }>(fetchPage: (view: QueueView, page: number) => Promise<Page<T>>) {
  const toast = useToast();
  const view = ref<QueueView>("open");
  const items = ref([]) as Ref<T[]>;
  const loaded = ref(false);
  const error = ref("");
  const hasMore = ref(false);
  const loadingMore = ref(false);
  let generation = 0;

  async function load() {
    const mine = ++generation;
    try {
      const page = await fetchPage(view.value, 1);
      if (mine !== generation) return;
      items.value = page.items;
      hasMore.value = page.has_more;
      error.value = "";
    } catch (err) {
      if (mine === generation) error.value = failed(err, t("admin.loadFailed"));
    } finally {
      if (mine === generation) loaded.value = true;
    }
  }

  async function more() {
    if (loadingMore.value || !hasMore.value) return;
    const mine = generation;
    loadingMore.value = true;
    try {
      const page = await fetchPage(view.value, nextQueuePage(items.value.length));
      if (mine !== generation) return;
      const shown = new Set(items.value.map((item) => item.id));
      items.value = [...items.value, ...page.items.filter((item) => !shown.has(item.id))];
      hasMore.value = page.has_more;
    } catch (err) {
      toast.show(failed(err, t("admin.loadFailed")));
    } finally {
      loadingMore.value = false;
    }
  }

  /** Switches the view and reads its first page; resolves once that is shown. */
  async function show(next: QueueView) {
    if (view.value === next) return;
    view.value = next;
    items.value = [];
    hasMore.value = false;
    loaded.value = false;
    error.value = "";
    await load();
  }

  /** Drops the listed items `gone` says a decision closed. */
  function drop(gone: (item: T) => boolean) {
    items.value = items.value.filter((item) => !gone(item));
  }

  return { view, items, loaded, error, hasMore, loadingMore, load, more, show, drop };
}
