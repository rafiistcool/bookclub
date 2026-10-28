import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EditionOut } from "../api/generated";

const editions = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { editions: (...args: unknown[]) => editions(...args) },
}));

import { editionLine } from "../editions";
import EditionPicker from "./EditionPicker.vue";

function edition(overrides: Partial<EditionOut> = {}): EditionOut {
  return {
    id: 1, work_id: 9, isbn13: null, isbn10: null, title: null, subtitle: null, publisher: "", language: null,
    format: null, pages: null, publication_year: null, planned_publication_date: null, contributors: [],
    cover_url: null, cover_thumb_url: null, cover_public_id: null, source: "dnb",
    ...overrides,
  } as EditionOut;
}

const PAPERBACK = edition({ id: 3, format: "paperback", publisher: "Penguin", publication_year: 2019, pages: 393, isbn13: "9780241315637" });
const HARDCOVER = edition({ id: 4, format: "hardcover", publisher: "Eichborn", title: "Ich bin Circe" });

describe("EditionPicker", () => {
  beforeEach(() => {
    editions.mockReset();
    editions.mockResolvedValue({ items: [PAPERBACK, HARDCOVER] });
  });

  it("describes an edition in one line", () => {
    expect(editionLine(PAPERBACK)).toBe("Paperback · Penguin · 2019 · 393 pages · ISBN 9780241315637");
    expect(editionLine(edition({ id: 7 }))).toBe("Edition 7");
  });

  it("asks for the editions only when the sheet opens, and chooses one", async () => {
    const wrapper = mount(EditionPicker, { props: { workId: 9, editionId: null }, global: { stubs: { teleport: true } } });
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
    const wrapper = mount(EditionPicker, { props: { workId: 9, editionId: 3 }, global: { stubs: { teleport: true } } });
    await flushPromises();
    expect(wrapper.get(".edition-current").text()).toContain("Paperback · Penguin");
    await wrapper.get(".edition-current .text-btn").trigger("click");
    await flushPromises();
    expect(editions).toHaveBeenCalledTimes(1);
    await wrapper.findAll(".edition-option")[0].trigger("click");
    expect(wrapper.emitted("choose")).toEqual([[null]]);
  });
});
