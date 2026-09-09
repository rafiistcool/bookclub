import type {
  BookDetail,
  CustomBookIn,
  ClubConfig,
  ClubPick,
  ClubPickBook,
  ClubPickCurrent,
  ColorMode,
  Diary,
  DiaryEntry,
  DiaryEntryIn,
  DiaryFeed,
  GoodreadsImport,
  Invite,
  OverlapList,
  Member,
  NextUpVote,
  ThemeId,
  VoteApplyResult,
  VoteBook,
  SearchPage,
  SearchParams,
  ShelfItem,
  ShelfList,
  User,
} from "../types";
import type { FinishNote, Status } from "../constants";

export class ApiError extends Error {
  status: number;
  detail: unknown;
  item?: ShelfItem;

  constructor(message: string, status: number, detail: unknown, item?: ShelfItem) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
    this.item = item;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const response = await fetch(path, {
    ...init,
    headers,
    credentials: "include",
  });
  if (response.status === 204) {
    return undefined as T;
  }
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const detail = data?.detail;
    const message =
      typeof detail === "string" ? detail : "Something went wrong. Try again.";
    throw new ApiError(message, response.status, detail ?? data, data?.item);
  }
  return data as T;
}

export const api = {
  config: () => request<ClubConfig>("/api/config"),
  me: () => request<User>("/api/auth/me"),
  register: (body: { username: string; password: string; invite_code: string }) =>
    request<User>("/api/auth/register", { method: "POST", body: JSON.stringify(body) }),
  login: (body: { username: string; password: string }) =>
    request<void>("/api/auth/login", { method: "POST", body: JSON.stringify(body) }),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),
  savePreferences: (body: { theme?: ThemeId; color_mode?: ColorMode }) =>
    request<User>("/api/auth/me/preferences", {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  search: (params: SearchParams = {}, init: RequestInit = {}) => {
    const query = new URLSearchParams();
    if (params.q) query.set("q", params.q);
    if (params.subject) query.set("subject", params.subject);
    if (params.sort) query.set("sort", params.sort);
    if (params.page) query.set("page", String(params.page));
    if (params.limit) query.set("limit", String(params.limit));
    const qs = query.toString();
    return request<SearchPage>(`/api/books/search${qs ? `?${qs}` : ""}`, init);
  },
  trending: (limit = 12, init: RequestInit = {}) =>
    request<SearchPage>(`/api/books/trending?limit=${limit}`, init),
  subject: (subject: string, page = 1, limit = 12, init: RequestInit = {}) =>
    request<SearchPage>(
      `/api/books/subjects/${encodeURIComponent(subject)}?page=${page}&limit=${limit}`,
      init,
    ),
  book: (workId: string, init: RequestInit = {}) =>
    request<BookDetail>(`/api/books/works/${workId}`, init),
  refreshBook: (workId: string) =>
    request<BookDetail>(`/api/books/works/${workId}/refresh`, { method: "POST" }),
  createCustomBook: (body: CustomBookIn) =>
    request<BookDetail>("/api/books/custom", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  myShelf: () => request<ShelfList>("/api/shelf"),
  friendShelf: (username: string) =>
    request<ShelfList>(`/api/shelf?username=${encodeURIComponent(username)}`),
  addToShelf: (body: {
    ol_work_key: string;
    title: string;
    authors: string;
    cover_id: number | null;
    year: number | null;
    status: Status;
  } & FinishNote) => request<ShelfItem>("/api/shelf", { method: "POST", body: JSON.stringify(body) }),
  patchShelf: (id: number, body: { status?: Status; position?: number } & FinishNote) =>
    request<ShelfItem>(`/api/shelf/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  removeFromShelf: (id: number) =>
    request<void>(`/api/shelf/${id}`, { method: "DELETE" }),
  importGoodreads: (file: File) => {
    const body = new FormData();
    body.append("file", file);
    return request<GoodreadsImport>("/api/shelf/import", { method: "POST", body });
  },
  members: () => request<Member[]>("/api/members"),
  invites: () => request<Invite[]>("/api/invites"),
  createInvite: () =>
    request<{ code: string }>("/api/invites", { method: "POST" }),
  clubPick: () => request<ClubPickCurrent>("/api/pick"),
  clubPickHistory: () => request<{ items: ClubPick[]; timezone: string }>("/api/pick/history"),
  setClubPick: (body: ClubPickBook) =>
    request<ClubPick>("/api/pick", { method: "PUT", body: JSON.stringify(body) }),
  clearClubPick: () => request<ClubPickCurrent>("/api/pick", { method: "DELETE" }),
  overlap: (includeReading = false) =>
    request<OverlapList>(
      `/api/overlap${includeReading ? "?include_reading=true" : ""}`,
    ),
  diary: (workId: string) => request<Diary>(`/api/books/works/${workId}/posts`),
  addDiaryEntry: (workId: string, body: DiaryEntryIn) =>
    request<DiaryEntry>(`/api/books/works/${workId}/posts`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  editDiaryEntry: (id: number, body: { body?: string; spoiler_upto?: number | null }) =>
    request<DiaryEntry>(`/api/posts/${id}`, { method: "PATCH", body: JSON.stringify(body) }),
  deleteDiaryEntry: (id: number) => request<void>(`/api/posts/${id}`, { method: "DELETE" }),
  reactToEntry: (id: number, emoji: string) =>
    request<DiaryEntry>(`/api/posts/${id}/reactions`, {
      method: "POST",
      body: JSON.stringify({ emoji }),
    }),
  diaryFeed: (before?: number | null, limit = 8) =>
    request<DiaryFeed>(`/api/diary?limit=${limit}${before ? `&before=${before}` : ""}`),
  nextUp: () => request<NextUpVote>("/api/vote"),
  nominate: (body: VoteBook) =>
    request<NextUpVote>("/api/vote/nominations", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  castVote: (nominationId: number) =>
    request<NextUpVote>("/api/vote/cast", {
      method: "POST",
      body: JSON.stringify({ nomination_id: nominationId }),
    }),
  applyWinner: (nominationId: number, meetingAt?: string | null) =>
    request<VoteApplyResult>("/api/vote/apply", {
      method: "POST",
      body: JSON.stringify({ nomination_id: nominationId, meeting_at: meetingAt ?? null }),
    }),
};
