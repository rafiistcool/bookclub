// The "Edit as admin" entry on the book page (#54, work-plan ADM scope 4). It tests
// pages/BookDetailPage.vue, which ADM owns, but lives here because its colocated test file
// pages/BookDetailPage.test.ts is outside ADM's files (docs/phase2/notes/adm.md).
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookDetail, edition, user } from "../../test/fixtures";
import type { BookDetail } from "../../types";

const book = vi.fn();
const editions = vi.fn();
const diary = vi.fn();
const requestWorkEdit = vi.fn();
const getEdition = vi.fn();
const adminEditWork = vi.fn();
const adminEditEdition = vi.fn();
const unused = vi.fn();

vi.mock("../../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(message: string, status: number) {
      super(message);
      this.name = "ApiError";
      this.status = status;
    }
  },
  api: {
    book: (...args: unknown[]) => book(...args),
    editions: (...args: unknown[]) => editions(...args),
    diary: (...args: unknown[]) => diary(...args),
    requestWorkEdit: (...args: unknown[]) => requestWorkEdit(...args),
    edition: (...args: unknown[]) => getEdition(...args),
    adminEditWork: (...args: unknown[]) => adminEditWork(...args),
    adminEditEdition: (...args: unknown[]) => adminEditEdition(...args),
    addFavorite: (...args: unknown[]) => unused(...args),
    removeFavorite: (...args: unknown[]) => unused(...args),
    addToShelf: (...args: unknown[]) => unused(...args),
    patchShelf: (...args: unknown[]) => unused(...args),
    removeFromShelf: (...args: unknown[]) => unused(...args),
    nominate: (...args: unknown[]) => unused(...args),
  },
}));

import { i18n } from "../../i18n";
import BookDetailPage from "../../pages/BookDetailPage.vue";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";

function detail(overrides: Partial<BookDetail> = {}): BookDetail {
  return bookDetail({
    id: 4,
    ol_work_key: null,
    title: "The Witch of Portobello",
    authors: "Paulo Coelho",
    year: 2006,
    source: "user",
    custom: true,
    can_refresh: false,
    ...overrides,
  });
}

async function mountBook(isAdmin: boolean) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/book/:id", component: BookDetailPage }],
  });
  await router.push("/book/4");
  await router.isReady();
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs: [], is_admin: isAdmin });
  const wrapper = mount(BookDetailPage, { global: { plugins: [pinia, router] } });
  await flushPromises();
  return wrapper;
}

function button(wrapper: Awaited<ReturnType<typeof mountBook>>, label: string) {
  return wrapper.findAll("button").find((node) => node.text().includes(label));
}

describe("BookDetailPage: edit as admin", () => {
  beforeEach(() => {
    for (const mock of [book, editions, diary, requestWorkEdit, getEdition, adminEditWork, adminEditEdition, unused]) {
      mock.mockReset();
    }
    diary.mockResolvedValue({ work_id: 4, my_progress: null, my_status: null, items: [], timezone: "UTC" });
    editions.mockResolvedValue({ items: [] });
    i18n.global.locale.value = "en";
  });

  it("lets only an instance admin edit the book and the reader's edition directly", async () => {
    book.mockResolvedValue(detail({ edition_id: 13 }));
    const reader = await mountBook(false);
    expect(button(reader, "as admin")).toBeUndefined();
    reader.unmount();

    const wrapper = await mountBook(true);
    await button(wrapper, "Edit as admin")!.trigger("click");
    const form = wrapper.get("form.edit-request");
    expect(wrapper.get(".sheet").text()).toContain("Edit this book as admin");
    expect(form.find("textarea[name=note]").exists()).toBe(false);
    await form.get("input[name=title]").setValue("The Witch of Portobello (revised)");
    adminEditWork.mockResolvedValue(detail({ edition_id: 13, title: "The Witch of Portobello (revised)" }));
    await form.trigger("submit");
    await flushPromises();
    expect(adminEditWork).toHaveBeenCalledWith(4, { title: "The Witch of Portobello (revised)" });
    expect(requestWorkEdit).not.toHaveBeenCalled();
    expect(wrapper.get("h1").text()).toBe("The Witch of Portobello (revised)");
    expect(wrapper.find("form.edit-request").exists()).toBe(false);

    getEdition.mockResolvedValue(edition({ id: 13, work_id: 4, publisher: "Diogenes" }));
    await button(wrapper, "Edit your edition as admin")!.trigger("click");
    await flushPromises();
    expect(getEdition).toHaveBeenCalledWith(13);
    const editionForm = wrapper.get("form.edit-request");
    expect(wrapper.get(".sheet").text()).toContain("Edit this edition as admin");
    await editionForm.get("input[name=publisher]").setValue("Diogenes Verlag");
    adminEditEdition.mockResolvedValue(edition({ id: 13, work_id: 4, publisher: "Diogenes Verlag" }));
    book.mockClear();
    await editionForm.trigger("submit");
    await flushPromises();
    expect(adminEditEdition).toHaveBeenCalledWith(13, { publisher: "Diogenes Verlag" });
    expect(book).toHaveBeenCalledWith(4);
    expect(useToast().items.at(-1)?.message).toBe("Saved. Everyone sees the new values now.");
    expect(unused).not.toHaveBeenCalled();
  });
});
