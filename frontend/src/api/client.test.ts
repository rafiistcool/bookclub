import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, ApiError } from "./client";

function mockFetch(status: number, body: unknown) {
  const text = body === undefined ? "" : JSON.stringify(body);
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(text),
  });
}

describe("api client", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = mockFetch(200, { ok: true });
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("sends JSON with credentials", async () => {
    await api.castVote(7);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/vote/cast");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.body).toBe(JSON.stringify({ nomination_id: 7 }));
    expect((init.headers as Headers).get("Content-Type")).toBe("application/json");
  });

  it("builds query strings, dropping empty values", async () => {
    await api.search({ q: "dune", subject: "", sort: "new", page: 2 });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/books/search?q=dune&sort=new&page=2");
    await api.search();
    expect(fetchMock.mock.calls[1][0]).toBe("/api/books/search");
    await api.trending(8);
    expect(fetchMock.mock.calls[2][0]).toBe("/api/books/trending?limit=8");
    await api.subject("science fiction", 2, 6);
    expect(fetchMock.mock.calls[3][0]).toBe(
      "/api/books/subjects/science%20fiction?page=2&limit=6",
    );
  });

  it("addresses book detail by bare work id", async () => {
    await api.book("OL1168007W");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/books/works/OL1168007W");
    await api.refreshBook("OL1168007W");
    expect(fetchMock.mock.calls[1][0]).toBe("/api/books/works/OL1168007W/refresh");
    expect(fetchMock.mock.calls[1][1].method).toBe("POST");
  });

  it("addresses the diary by work id and entries by id", async () => {
    await api.diary("OL1W");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/books/works/OL1W/posts");
    await api.addDiaryEntry("OL1W", { body: "hi", spoiler_upto: 40, parent_id: 7 });
    expect(fetchMock.mock.calls[1][0]).toBe("/api/books/works/OL1W/posts");
    expect(fetchMock.mock.calls[1][1].body).toBe(
      JSON.stringify({ body: "hi", spoiler_upto: 40, parent_id: 7 }),
    );
    await api.editDiaryEntry(7, { body: "edited" });
    expect(fetchMock.mock.calls[2][0]).toBe("/api/posts/7");
    expect(fetchMock.mock.calls[2][1].method).toBe("PATCH");
    await api.deleteDiaryEntry(7);
    expect(fetchMock.mock.calls[3][1].method).toBe("DELETE");
    await api.reactToEntry(7, "❤️");
    expect(fetchMock.mock.calls[4][0]).toBe("/api/posts/7/reactions");
    await api.diaryFeed();
    expect(fetchMock.mock.calls[5][0]).toBe("/api/diary?limit=8");
    await api.diaryFeed(42, 5);
    expect(fetchMock.mock.calls[6][0]).toBe("/api/diary?limit=5&before=42");
  });

  it("patches theme preferences", async () => {
    await api.savePreferences({ theme: "ink" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/auth/me/preferences");
    expect(init.method).toBe("PATCH");
    expect(init.body).toBe(JSON.stringify({ theme: "ink" }));
  });

  it("returns undefined on 204 and parses JSON otherwise", async () => {
    vi.stubGlobal("fetch", mockFetch(204, undefined));
    expect(await api.logout()).toBeUndefined();
    vi.stubGlobal("fetch", mockFetch(200, { name: "Club" }));
    expect(await api.config()).toEqual({ name: "Club" });
  });

  it("throws ApiError with detail and the conflicting shelf item", async () => {
    const item = { id: 9, status: "finished" };
    vi.stubGlobal("fetch", mockFetch(409, { detail: "Already on your shelf", item }));
    await expect(
      api.addToShelf({
        ol_work_key: "/works/OL1W",
        title: "x",
        authors: "",
        cover_id: null,
        year: null,
        status: "want_to_read",
      }),
    ).rejects.toMatchObject({ name: "ApiError", status: 409, message: "Already on your shelf", item });
  });

  it("falls back to a generic message when detail is structured", async () => {
    vi.stubGlobal("fetch", mockFetch(500, { detail: { code: 1 } }));
    const err = await api.me().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).message).toBe("Something went wrong. Try again.");
  });

  it("does not set a JSON content type for form uploads", async () => {
    await api.importGoodreads(new File(["a,b"], "x.csv"));
    const init = fetchMock.mock.calls[0][1];
    expect(init.body).toBeInstanceOf(FormData);
    expect((init.headers as Headers).has("Content-Type")).toBe(false);
  });
});
