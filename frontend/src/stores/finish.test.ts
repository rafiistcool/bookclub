import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it } from "vitest";
import { shelfItem } from "../test/fixtures";
import { isFinishTransition, momentPatch, useFinish } from "./finish";

describe("isFinishTransition", () => {
  it("is true only when the book was not finished before", () => {
    const finished = shelfItem({ status: "finished" });
    expect(isFinishTransition("currently_reading", finished)).toBe(true);
    expect(isFinishTransition(null, finished)).toBe(true);
    expect(isFinishTransition(undefined, finished)).toBe(true);
    expect(isFinishTransition("finished", finished)).toBe(false);
    expect(isFinishTransition("want_to_read", shelfItem({ status: "currently_reading" }))).toBe(false);
  });
});

describe("momentPatch", () => {
  const item = shelfItem({ status: "finished", rating: null, take: "", rating_visibility: "audience" });

  it("sends nothing when the reader set nothing", () => {
    expect(momentPatch(item, { rating: null, take: "  ", onlyMe: false })).toBeNull();
  });

  it("sends only the fields that were set, the take trimmed", () => {
    expect(momentPatch(item, { rating: 4, take: "", onlyMe: false })).toEqual({ rating: 4 });
    expect(momentPatch(item, { rating: null, take: " Kind. ", onlyMe: false })).toEqual({ take: "Kind." });
    expect(momentPatch(item, { rating: 5, take: "Sharp.", onlyMe: true })).toEqual({
      rating: 5,
      take: "Sharp.",
      rating_visibility: "only_me",
    });
  });

  it("leaves out what the entry already stores", () => {
    const stored = shelfItem({ status: "finished", rating: 3, take: "Yes.", rating_visibility: "only_me" });
    expect(momentPatch(stored, { rating: 3, take: "Yes.", onlyMe: true })).toBeNull();
    expect(momentPatch(stored, { rating: 3, take: "Yes.", onlyMe: false })).toEqual({ rating_visibility: "audience" });
  });
});

describe("useFinish", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("opens for a finished entry and resolves with what the moment saved", async () => {
    const finish = useFinish();
    const item = shelfItem({ id: 7, status: "finished" });
    const answer = finish.celebrate({ item, clubSlug: "readers" });
    expect(finish.item).toStrictEqual(item);
    expect(finish.clubSlug).toBe("readers");
    const saved = { ...item, rating: 5 };
    finish.close(saved);
    expect(finish.item).toBeNull();
    expect(finish.lastSaved).toStrictEqual(saved);
    await expect(answer).resolves.toStrictEqual(saved);
  });

  it("never opens for an entry that is not finished", async () => {
    const finish = useFinish();
    await expect(finish.celebrate({ item: shelfItem({ status: "currently_reading" }) })).resolves.toBeNull();
    expect(finish.item).toBeNull();
  });

  it("resolves null on skip and keeps the last saved entry", async () => {
    const finish = useFinish();
    const answer = finish.celebrate({ item: shelfItem({ status: "finished" }) });
    expect(finish.clubSlug).toBeNull();
    finish.close();
    await expect(answer).resolves.toBeNull();
    expect(finish.lastSaved).toBeNull();
    // Closing with nothing open is a no-op.
    finish.close();
  });

  it("lets a newer finish take the place of an open one", async () => {
    const finish = useFinish();
    const first = finish.celebrate({ item: shelfItem({ id: 1, status: "finished" }) });
    const second = finish.celebrate({ item: shelfItem({ id: 2, status: "finished" }) });
    await expect(first).resolves.toBeNull();
    expect(finish.item?.id).toBe(2);
    finish.close();
    await expect(second).resolves.toBeNull();
  });

  it("closes only the moment a late save belongs to", async () => {
    const finish = useFinish();
    finish.celebrate({ item: shelfItem({ id: 1, status: "finished" }) });
    const first = finish.serial;
    // Its "Save" is still saving when another finish opens the next moment.
    const second = finish.celebrate({ item: shelfItem({ id: 2, status: "finished" }) });
    const savedFirst = shelfItem({ id: 1, status: "finished", rating: 4 });
    finish.close(savedFirst, first);
    expect(finish.item?.id).toBe(2);
    // The page of the first book still learns its stars.
    expect(finish.lastSaved).toStrictEqual(savedFirst);
    const savedSecond = shelfItem({ id: 2, status: "finished", rating: 5 });
    finish.close(savedSecond, finish.serial);
    expect(finish.item).toBeNull();
    await expect(second).resolves.toStrictEqual(savedSecond);
  });

  it("notes where focus goes back, while the opener is still on the page", () => {
    const finish = useFinish();
    const row = document.createElement("li");
    const button = document.createElement("button");
    row.append(button);
    document.body.append(row);
    finish.celebrate({ item: shelfItem({ status: "finished" }), opener: button });
    row.remove();
    expect(finish.returnFocus).toEqual([button, row]);
    finish.close();

    document.body.append(row);
    button.focus();
    finish.celebrate({ item: shelfItem({ status: "finished" }) });
    expect(finish.returnFocus[0]).toBe(button);
    row.remove();
  });
});
