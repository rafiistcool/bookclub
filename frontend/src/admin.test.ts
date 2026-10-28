import { afterEach, describe, expect, it } from "vitest";
import { mergeTarget, proposedFields, requestKindLabel } from "./admin";
import { i18n } from "./i18n";
import { languageName, languageOptions } from "./languages";

const request = (kind: string, payload: Record<string, unknown>) => ({
  kind,
  payload: payload as Record<string, never>,
});

describe("the admin queue", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  it("labels each proposed field and marks a cleared one", () => {
    const fields = proposedFields(
      request("edition_edit", { pages: 401, format: "paperback", language: "ger", subtitle: null, subjects: ["A", "B"] }),
      "en",
    );
    const unknown = { before: null, wasEmpty: false, same: false };
    expect(fields).toEqual([
      { key: "pages", label: "Pages", value: "401", cleared: false, ...unknown },
      { key: "format", label: "Format", value: "Paperback", cleared: false, ...unknown },
      { key: "language", label: "Language", value: "German", cleared: false, ...unknown },
      { key: "subtitle", label: "Subtitle", value: "(clear it)", cleared: true, ...unknown },
      { key: "subjects", label: "Subjects", value: "A, B", cleared: false, ...unknown },
    ]);
    expect(proposedFields(request("cover_replace", {}), "en")).toEqual([]);
  });

  it("puts the record's value now beside each proposed one", () => {
    const fields = proposedFields(
      {
        ...request("edition_edit", { pages: 401, format: "paperback", language: "ger", subtitle: null, publisher: "Klett" }),
        current: { pages: 380, format: null, language: "eng", subtitle: "Roman", publisher: "Klett" } as never,
      },
      "en",
    );
    expect(fields.map(({ key, before, wasEmpty, same }) => ({ key, before, wasEmpty, same }))).toEqual([
      { key: "pages", before: "380", wasEmpty: false, same: false },
      { key: "format", before: "(empty)", wasEmpty: true, same: false },
      { key: "language", before: "English", wasEmpty: false, same: false },
      { key: "subtitle", before: "Roman", wasEmpty: false, same: false },
      { key: "publisher", before: "Klett", wasEmpty: false, same: true },
    ]);
    i18n.global.locale.value = "de";
    const [subjects] = proposedFields(
      { ...request("work_edit", { subjects: [] }), current: { subjects: [] } as never },
      "de",
    );
    expect(subjects).toMatchObject({ value: "(leeren)", before: "(leer)", wasEmpty: true, same: true });
  });

  it("names the request kinds in both languages, and the merge target", () => {
    expect(requestKindLabel("work_merge")).toBe("Same book");
    i18n.global.locale.value = "de";
    expect(requestKindLabel("cover_replace")).toBe("Neues Cover-Foto");
    expect(requestKindLabel("something_new")).toBe("something_new");
    expect(mergeTarget(request("work_merge", { into_work_id: 31 }))).toBe(31);
    expect(mergeTarget(request("work_edit", { into_work_id: 31 }))).toBeNull();
  });

  it("names languages by their catalog code in the UI language", () => {
    expect(languageName("ger", "de")).toBe("Deutsch");
    expect(languageName("fre", "en")).toBe("French");
    expect(languageName("xyz", "en")).toBe("xyz");
    const options = languageOptions("en", "xyz");
    expect(options.map((row) => row.code)).toContain("xyz");
    expect(options[0]!.name.localeCompare(options[1]!.name)).toBeLessThan(0);
  });
});
