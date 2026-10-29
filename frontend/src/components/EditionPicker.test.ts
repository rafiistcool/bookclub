import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { edition } from "../test/fixtures";

const editions = vi.fn();
const addEdition = vi.fn();
const requestEditionEdit = vi.fn();
const reportEdition = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    body: unknown;
    constructor(message: string, status: number, _detail?: unknown, body?: unknown) {
      super(message);
      this.status = status;
      this.body = body ?? null;
    }
  },
  api: {
    editions: (...args: unknown[]) => editions(...args),
    addEdition: (...args: unknown[]) => addEdition(...args),
    requestEditionEdit: (...args: unknown[]) => requestEditionEdit(...args),
    reportEdition: (...args: unknown[]) => reportEdition(...args),
  },
}));

import { ApiError } from "../api/client";
import { editionLine } from "../editions";
import { useToast } from "../stores/toast";
import EditionPicker from "./EditionPicker.vue";

function mountPicker(editionId: number | null) {
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:rest(.*)*", component: { template: "<p />" } }] });
  return mount(EditionPicker, {
    props: { workId: 9, editionId },
    global: { plugins: [router], stubs: { teleport: true } },
  });
}

async function openSheet(wrapper: ReturnType<typeof mountPicker>) {
  await wrapper.get(".edition-current .text-btn").trigger("click");
  await flushPromises();
}

const PAPERBACK = edition({ id: 3, format: "paperback", publisher: "Penguin", publication_year: 2019, pages: 393, isbn13: "9780241315637" });
const HARDCOVER = edition({ id: 4, format: "hardcover", publisher: "Eichborn", title: "Ich bin Circe" });

describe("EditionPicker", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    addEdition.mockReset();
    requestEditionEdit.mockReset();
    reportEdition.mockReset();
    editions.mockReset();
    editions.mockResolvedValue({ items: [PAPERBACK, HARDCOVER] });
  });

  it("describes an edition in one line", () => {
    expect(editionLine(PAPERBACK)).toBe("Paperback · Penguin · 2019 · 393 pages · ISBN 9780241315637");
    expect(editionLine(edition({ id: 7 }))).toBe("Edition 7");
  });

  it("asks for the editions only when the sheet opens, and chooses one", async () => {
    const wrapper = mountPicker(null);
    await flushPromises();
    expect(editions).not.toHaveBeenCalled();
    expect(wrapper.get(".edition-current").text()).toContain("No particular edition");

    await wrapper.get(".edition-current .text-btn").trigger("click");
    await flushPromises();
    expect(editions).toHaveBeenCalledWith(9);
    const options = wrapper.findAll(".edition-option");
    expect(options.map((row) => row.text())).toEqual([
      "No particular edition",
      "Paperback · Penguin · 2019 · 393 pages · ISBN 9780241315637",
      "Ich bin CirceHardcover · Eichborn",
    ]);
    expect(options[0].attributes("aria-pressed")).toBe("true");
    await options[2].trigger("click");
    expect(wrapper.emitted("choose")).toEqual([[4]]);
  });

  it("names the chosen edition, and goes back to none", async () => {
    const wrapper = mountPicker(3);
    await flushPromises();
    expect(wrapper.get(".edition-current").text()).toContain("Paperback · Penguin");
    await wrapper.get(".edition-current .text-btn").trigger("click");
    await flushPromises();
    expect(editions).toHaveBeenCalledTimes(1);
    await wrapper.findAll(".edition-option")[0].trigger("click");
    expect(wrapper.emitted("choose")).toEqual([[null]]);
  });

  it("adds a missing edition and makes it the reader's", async () => {
    const wrapper = mountPicker(null);
    await openSheet(wrapper);
    await wrapper.get(".edition-missing").trigger("click");
    const form = wrapper.get("form.add-edition");
    await form.get("select[name=format]").setValue("paperback");
    await form.get("select[name=language]").setValue("ger");
    await form.get("input[name=publisher]").setValue("  dtv ");
    await form.get("input[name=year]").setValue("2021");
    await form.get("input[name=pages]").setValue("480");
    const added = edition({ id: 12, format: "paperback", publisher: "dtv", publication_year: 2021, pages: 480, language: "ger", source: "user" });
    addEdition.mockResolvedValue(added);
    await form.trigger("submit");
    await flushPromises();
    expect(addEdition).toHaveBeenCalledWith(9, {
      isbn: null,
      format: "paperback",
      publisher: "dtv",
      publication_year: 2021,
      pages: 480,
      language: "ger",
      title: null,
    });
    expect(wrapper.emitted("choose")).toEqual([[12]]);
    expect(useToast().items.at(-1)?.message).toBe("Edition added");
  });

  it("asks for something before adding, and offers the edition an ISBN already names", async () => {
    const wrapper = mountPicker(null);
    await openSheet(wrapper);
    await wrapper.get(".edition-missing").trigger("click");
    const form = wrapper.get("form.add-edition");
    await form.trigger("submit");
    expect(addEdition).not.toHaveBeenCalled();
    expect(form.get(".error").text()).toBe("Fill in at least one field.");

    // The ISBN is an edition of this book: choose it.
    await form.get("input[name=isbn]").setValue("978-3-423-28254-1");
    addEdition.mockRejectedValue(
      new ApiError("That ISBN is already in the catalog", 409, "That ISBN is already in the catalog", {
        detail: "That ISBN is already in the catalog",
        work_id: 9,
        edition_id: 4,
      }),
    );
    await form.trigger("submit");
    await flushPromises();
    expect(addEdition.mock.calls[0]![1]).toMatchObject({ isbn: "978-3-423-28254-1" });
    await wrapper.get("form.add-edition .error .text-btn").trigger("click");
    expect(wrapper.emitted("choose")).toEqual([[4]]);

    // Of another book: open that one.
    await openSheet(wrapper);
    await wrapper.get(".edition-missing").trigger("click");
    await wrapper.get("form.add-edition input[name=isbn]").setValue("9780241315637");
    addEdition.mockRejectedValue(
      new ApiError("That ISBN is already in the catalog", 409, "That ISBN is already in the catalog", {
        detail: "That ISBN is already in the catalog",
        work_id: 31,
        edition_id: 70,
      }),
    );
    await wrapper.get("form.add-edition").trigger("submit");
    await flushPromises();
    expect(wrapper.get("form.add-edition .error a").attributes("href")).toBe("/book/31");
  });

  it("suggests a correction of the chosen edition with only the changed fields", async () => {
    const wrapper = mountPicker(3);
    await flushPromises();
    await wrapper.get(".edition-correct").trigger("click");
    const form = wrapper.get("form.edit-request");
    expect((form.get("input[name=publisher]").element as HTMLInputElement).value).toBe("Penguin");
    await form.get("input[name=pages]").setValue("401");
    await form.get("select[name=language]").setValue("eng");
    await form.get("textarea[name=note]").setValue("Counted them.");
    requestEditionEdit.mockResolvedValue({ id: 5, status: "open" });
    await form.trigger("submit");
    await flushPromises();
    expect(requestEditionEdit).toHaveBeenCalledWith(3, { language: "eng", pages: 401 }, "Counted them.");
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.find("form.edit-request").exists()).toBe(false);
  });

  it("offers a report on each edition a reader added, like the correction sheet", async () => {
    const READERS = edition({ id: 12, format: "paperback", publisher: "Selbstverlag", source: "user" });
    editions.mockResolvedValue({ items: [PAPERBACK, READERS, HARDCOVER] });
    const wrapper = mountPicker(null);
    await openSheet(wrapper);
    const rows = wrapper.findAll(".edition-row");
    expect(rows.map((row) => row.find(".edition-report").exists())).toEqual([false, true, false]);
    const button = rows[1].get(".edition-report");
    expect(button.text()).toBe("Report");
    expect(button.attributes("aria-label")).toBe("Report the edition Paperback · Selbstverlag");

    await button.trigger("click");
    await flushPromises();
    expect(wrapper.find(".edition-list").exists()).toBe(false);
    expect(wrapper.emitted("choose")).toBeUndefined();
    expect(wrapper.text()).toContain("Report this edition");
    expect(wrapper.text()).toContain("Paperback · Selbstverlag");
    const reasons = wrapper.findAll("input[name=reason]").map((input) => input.attributes("value"));
    expect(reasons).toEqual(["offensive", "copyright", "other"]);
    await wrapper.get("input[value=copyright]").setValue(true);
    reportEdition.mockResolvedValue({ id: 8, status: "open" });
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(reportEdition).toHaveBeenCalledWith(12, { reason: "copyright", details: null });
    expect(useToast().items.at(-1)?.message).toBe("Thanks. An admin will look at it.");
    expect(wrapper.find("input[name=reason]").exists()).toBe(false);
  });

  it("names an edition by its own title in the report, as the admin queue and the mails do", async () => {
    const TITLED = edition({
      id: 14,
      title: "Probeheft: Sonderausgabe",
      format: "paperback",
      publisher: "Probeverlag Leuchtkäfer",
      source: "user",
    });
    editions.mockResolvedValue({ items: [TITLED] });
    const wrapper = mountPicker(null);
    await openSheet(wrapper);
    const button = wrapper.get(".edition-report");
    expect(button.attributes("aria-label")).toBe(
      "Report the edition Probeheft: Sonderausgabe · Paperback · Probeverlag Leuchtkäfer",
    );
    await button.trigger("click");
    await flushPromises();
    expect(wrapper.get(".report-subject").text()).toBe("Probeheft: Sonderausgabe · Paperback · Probeverlag Leuchtkäfer");
  });
});
