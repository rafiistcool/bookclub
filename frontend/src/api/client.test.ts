import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, ApiError, setUnauthorizedHandler } from "./client";

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
    setUnauthorizedHandler(null);
  });

  it("sends JSON with credentials", async () => {
    await api.nominate("book-club-ab12", 7);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/clubs/book-club-ab12/vote/nominations");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.body).toBe(JSON.stringify({ work_id: 7 }));
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
    const controller = new AbortController();
    await api.search({ q: "circe" }, { signal: controller.signal });
    expect(fetchMock.mock.calls[4][1].signal).toBe(controller.signal);
    await api.newReleases(1, 14);
    expect(fetchMock.mock.calls[5][0]).toBe("/api/books/new-releases?language=ger&page=1&limit=14");
  });

  it("addresses book detail by numeric work id, or a legacy key to resolve", async () => {
    await api.book(4);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/books/works/4");
    await api.book("OL1168007W");
    expect(fetchMock.mock.calls[1][0]).toBe("/api/books/works/OL1168007W");
    await api.refreshBook(4);
    expect(fetchMock.mock.calls[2][0]).toBe("/api/books/works/4/refresh");
    expect(fetchMock.mock.calls[2][1].method).toBe("POST");
    await api.createCustomBook({ title: "My Zine", authors: "Ada" });
    expect(fetchMock.mock.calls[3][0]).toBe("/api/books/custom");
    expect(fetchMock.mock.calls[3][1].method).toBe("POST");
  });

  it("shelves a book by work id, without any book metadata", async () => {
    await api.addToShelf({ work_id: 4, status: "finished", rating: 5 });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/shelf");
    expect(fetchMock.mock.calls[0][1].body).toBe(JSON.stringify({ work_id: 4, status: "finished", rating: 5 }));
    await api.importJob(7);
    expect(fetchMock.mock.calls[1][0]).toBe("/api/shelf/import/7");
  });

  it("reports a cover by its media id", async () => {
    await api.reportCover("abc2def", { reason: "wrong_book", details: null });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/media/abc2def/reports");
    expect(init.method).toBe("POST");
    expect(init.body).toBe(JSON.stringify({ reason: "wrong_book", details: null }));
  });

  it("addresses the diary by work id and entries by id", async () => {
    await api.diary(1);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/books/works/1/posts");
    await api.addDiaryEntry(1, { body: "hi", spoiler_upto: 40, parent_id: 7 });
    expect(fetchMock.mock.calls[1][0]).toBe("/api/books/works/1/posts");
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
    await api.diaryFeed(null, 8, "book-club-ab12");
    expect(fetchMock.mock.calls[7][0]).toBe("/api/diary?limit=8&club=book-club-ab12");
  });

  it("replaces and toggles favourites", async () => {
    await api.replaceFavorites([3, 1]);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/auth/me/favorites");
    expect(fetchMock.mock.calls[0][1].method).toBe("PUT");
    expect(fetchMock.mock.calls[0][1].body).toBe(JSON.stringify({ work_ids: [3, 1] }));
    await api.myFavorites();
    expect(fetchMock.mock.calls[1][0]).toBe("/api/auth/me/favorites");
    await api.addFavorite(9);
    expect(fetchMock.mock.calls[2][0]).toBe("/api/auth/me/favorites/9");
    expect(fetchMock.mock.calls[2][1].method).toBe("POST");
    await api.removeFavorite(9);
    expect(fetchMock.mock.calls[3][0]).toBe("/api/auth/me/favorites/9");
    expect(fetchMock.mock.calls[3][1].method).toBe("DELETE");
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
    await expect(api.addToShelf({ work_id: 1, status: "want_to_read" })).rejects.toMatchObject({
      name: "ApiError",
      status: 409,
      message: "That book is already on your shelf",
      detail: "Already on your shelf",
      item,
    });
  });

  it("keeps the running import of a 409 and the rest of the body", async () => {
    const job = { id: 3, status: "running" };
    vi.stubGlobal("fetch", mockFetch(409, { detail: "An import is already running", job }));
    const err = await api.importGoodreads(new File(["a"], "x.csv")).catch((e: unknown) => e);
    expect(err).toMatchObject({ status: 409, job, body: { detail: "An import is already running", job } });
    expect((err as ApiError).message).toBe("An import is already running");
  });

  it("reports a 401 as an ended session, except for a wrong password at login", async () => {
    const ended = vi.fn();
    setUnauthorizedHandler(ended);
    vi.stubGlobal("fetch", mockFetch(401, { detail: "Not signed in" }));
    await expect(api.myShelf()).rejects.toMatchObject({ status: 401 });
    expect(ended).toHaveBeenCalledTimes(1);
    vi.stubGlobal("fetch", mockFetch(401, { detail: "Wrong username or password" }));
    await expect(api.login({ username: "ada", password: "nope" })).rejects.toMatchObject({ status: 401 });
    expect(ended).toHaveBeenCalledTimes(1);
    vi.stubGlobal("fetch", mockFetch(403, { detail: "Only club admins can do that" }));
    await expect(api.setClubPick("c-1", { work_id: 1 })).rejects.toMatchObject({ status: 403 });
    expect(ended).toHaveBeenCalledTimes(1);
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
