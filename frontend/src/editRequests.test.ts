import { describe, expect, it } from "vitest";
import { changedFields, EDITION_FIELDS, editionDraft, WORK_FIELDS, workDraft } from "./editRequests";
import { bookDetail, edition } from "./test/fixtures";

describe("edit requests", () => {
  const circe = bookDetail({
    id: 9,
    title: "Circe",
    subtitle: "",
    authors: "Madeline Miller",
    year: 2018,
    pages: null,
    description: "A witch.",
    subjects: ["Mythology", "Women, history"],
  });

  it("sends nothing for an untouched form", () => {
    const before = workDraft(circe);
    expect(changedFields(WORK_FIELDS, before, { ...before })).toEqual({ fields: {}, invalid: [] });
  });

  it("sends only what changed, in the types the server applies", () => {
    const before = workDraft(circe);
    const after = {
      ...before,
      title: "  Circe ",
      subtitle: "A Novel",
      year: 2019,
      pages: "393",
      description: "",
      subjects: "Mythology\nWomen, history\n\nGreek gods ",
    };
    expect(changedFields(WORK_FIELDS, before, after)).toEqual({
      fields: {
        subtitle: "A Novel",
        year: 2019,
        pages: 393,
        description: null,
        subjects: ["Mythology", "Women, history", "Greek gods"],
      },
      invalid: [],
    });
  });

  it("keeps a subject with a comma in it whole", () => {
    expect(workDraft(circe).subjects).toBe("Mythology\nWomen, history");
  });

  it("names the fields the server would refuse instead of sending them", () => {
    const before = workDraft(circe);
    const after = { ...before, title: "  ", year: "2018.5", pages: 0 };
    expect(changedFields(WORK_FIELDS, before, after)).toEqual({ fields: {}, invalid: ["title", "year", "pages"] });
  });

  it("clears an edition's format and language with null", () => {
    const paperback = edition({ id: 3, format: "paperback", language: "eng", publisher: "Penguin", pages: 393 });
    const before = editionDraft(paperback);
    expect(before).toMatchObject({ format: "paperback", language: "eng", pages: "393", title: "" });
    const after = { ...before, format: "", language: "ger", publication_year: 2021 };
    expect(changedFields(EDITION_FIELDS, before, after).fields).toEqual({
      format: null,
      language: "ger",
      publication_year: 2021,
    });
  });
});
