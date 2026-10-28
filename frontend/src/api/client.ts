import type {
  BookDetailOut,
  ClubList,
  ClubPickCurrentOut,
  ClubPickHistoryOut,
  ClubPickOut,
  ClubPickPatchIn,
  ClubPickSetIn,
  CustomBookIn,
  DiaryEntryIn,
  DiaryFeedOut,
  DiaryOut,
  FavoritesOut,
  ImportJobOut,
  LoginIn,
  MemberOut,
  OverlapOut,
  PostOut,
  PostPatchIn,
  PreferencesIn,
  RegisterIn,
  ReportCreated,
  ReportIn,
  SearchHistoryOut,
  SearchPage,
  ServiceConfig,
  ShelfAddIn,
  ShelfItemOut,
  ShelfListOut,
  ShelfPatchIn,
  SpineColorsOut,
  UserOut,
  VoteApplyOut,
  VoteOut,
} from "./generated";
import type { SearchParams } from "../types";
import { t } from "../i18n";
import { translateApiError } from "../i18n/apiErrors";
import type { PhysicalShelfLayout } from "../physicalShelf";

function field(body: unknown, key: string): unknown {
  return body !== null && typeof body === "object" ? (body as Record<string, unknown>)[key] : undefined;
}

export class ApiError extends Error {
  status: number;
  /** The server's English `detail`, or the whole body when it has none. */
  detail: unknown;
  /** The parsed error body. A few 409s carry more than `detail`. */
  body: unknown;
  /** `POST /api/shelf` 409: the entry that is already on the shelf. */
  item?: ShelfItemOut;
  /** `POST /api/shelf/import` 409: the import that is already running. */
  job?: ImportJobOut;

  constructor(message: string, status: number, detail: unknown, body: unknown = null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
    this.body = body;
    this.item = field(body, "item") as ShelfItemOut | undefined;
    this.job = field(body, "job") as ImportJobOut | undefined;
  }
}

/**
 * Called when a request that needed a session answers 401: the session ended
 * (logout elsewhere, a password reset, a deleted account). The app installs a
 * handler that resets the session and sends the user to the login page.
 */
let unauthorizedHandler: (() => void) | null = null;

export function setUnauthorizedHandler(handler: (() => void) | null) {
  unauthorizedHandler = handler;
}

/** These answer 401 for wrong credentials, not for a missing session. */
const CREDENTIAL_CHECKS = ["/api/auth/login", "/api/auth/tokens"];

/**
 * The one place request headers are built. The apps (Phase 4) add their
 * `Authorization: Bearer …` header here; the web app relies on its cookie.
 */
function requestHeaders(init: RequestInit): Headers {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  return headers;
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: requestHeaders(init),
    credentials: "include",
  });
  if (response.status === 204) {
    return undefined as T;
  }
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;
  if (!response.ok) {
    if (response.status === 401 && !CREDENTIAL_CHECKS.includes(path)) {
      unauthorizedHandler?.();
    }
    const detail = field(data, "detail");
    const message =
      typeof detail === "string" ? translateApiError(detail) : t("errors.generic");
    throw new ApiError(message, response.status, detail ?? data, data);
  }
  return data as T;
}

function json(method: string, body: unknown): RequestInit {
  return { method, body: JSON.stringify(body) };
}

function upload(file: File): FormData {
  const body = new FormData();
  body.append("file", file);
  return body;
}

const enc = encodeURIComponent;

/** `{work_id}` path segment: a numeric work id, or a legacy key such as `OL1W` or `BCdeadbeef01`. */
function workPath(workId: number | string): string {
  return `/api/books/works/${enc(String(workId))}`;
}

function clubPath(slug: string): string {
  return `/api/clubs/${enc(slug)}`;
}

export const api = {
  config: () => request<ServiceConfig>("/api/config"),
  me: () => request<UserOut>("/api/auth/me"),
  register: (body: RegisterIn) => request<UserOut>("/api/auth/register", json("POST", body)),
  login: (body: LoginIn) => request<void>("/api/auth/login", json("POST", body)),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),
  savePreferences: (body: PreferencesIn) =>
    request<UserOut>("/api/auth/me/preferences", json("PATCH", body)),
  uploadAvatar: (file: File) =>
    request<UserOut>("/api/auth/me/avatar", { method: "PUT", body: upload(file) }),
  deleteAvatar: () => request<UserOut>("/api/auth/me/avatar", { method: "DELETE" }),

  myFavorites: () => request<FavoritesOut>("/api/auth/me/favorites"),
  replaceFavorites: (workIds: number[]) =>
    request<FavoritesOut>("/api/auth/me/favorites", json("PUT", { work_ids: workIds })),
  addFavorite: (workId: number) =>
    request<FavoritesOut>(`/api/auth/me/favorites/${workId}`, { method: "POST" }),
  removeFavorite: (workId: number) =>
    request<FavoritesOut>(`/api/auth/me/favorites/${workId}`, { method: "DELETE" }),

  searchHistory: () => request<SearchHistoryOut[]>("/api/search-history"),
  rememberSearch: (query: string) =>
    request<SearchHistoryOut[]>("/api/search-history", json("POST", { query })),
  deleteSearchHistory: (id?: number) => request<void>(
    id === undefined ? "/api/search-history" : `/api/search-history/${id}`, { method: "DELETE" },
  ),

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
    request<SearchPage>(`/api/books/subjects/${enc(subject)}?page=${page}&limit=${limit}`, init),
  /** Upcoming and recent German titles from the DNB harvest (local only). */
  newReleases: (page = 1, limit = 24, init: RequestInit = {}) =>
    request<SearchPage>(`/api/books/new-releases?language=ger&page=${page}&limit=${limit}`, init),
  book: (workId: number | string, init: RequestInit = {}) =>
    request<BookDetailOut>(workPath(workId), init),
  refreshBook: (workId: number) =>
    request<BookDetailOut>(`${workPath(workId)}/refresh`, { method: "POST" }),
  createCustomBook: (body: CustomBookIn) =>
    request<BookDetailOut>("/api/books/custom", json("POST", body)),
  /** Notice-and-takedown for a cover, by `cover_public_id`. */
  reportCover: (publicId: string, body: ReportIn) =>
    request<ReportCreated>(`/api/media/${enc(publicId)}/reports`, json("POST", body)),

  myShelf: () => request<ShelfListOut>("/api/shelf"),
  friendShelf: (username: string) => request<ShelfListOut>(`/api/shelf?username=${enc(username)}`),
  addToShelf: (body: ShelfAddIn) => request<ShelfItemOut>("/api/shelf", json("POST", body)),
  patchShelf: (id: number, body: ShelfPatchIn) =>
    request<ShelfItemOut>(`/api/shelf/${id}`, json("PATCH", body)),
  removeFromShelf: (id: number) => request<void>(`/api/shelf/${id}`, { method: "DELETE" }),
  physicalShelfLayout: (username?: string) =>
    request<PhysicalShelfLayout>(
      `/api/shelf/physical-layout${username ? `?username=${enc(username)}` : ""}`,
    ),
  savePhysicalShelfLayout: (layout: PhysicalShelfLayout) =>
    request<PhysicalShelfLayout>("/api/shelf/physical-layout", json("PUT", layout)),
  spineColors: () => request<SpineColorsOut>("/api/shelf/spine-colors", { method: "POST" }),
  /** Starts a background import: 202 with the queued job. */
  importGoodreads: (file: File) =>
    request<ImportJobOut>("/api/shelf/import", { method: "POST", body: upload(file) }),
  importJob: (id: number) => request<ImportJobOut>(`/api/shelf/import/${id}`),

  diary: (workId: number | string) => request<DiaryOut>(`${workPath(workId)}/posts`),
  addDiaryEntry: (workId: number | string, body: DiaryEntryIn) =>
    request<PostOut>(`${workPath(workId)}/posts`, json("POST", body)),
  editDiaryEntry: (id: number, body: PostPatchIn) =>
    request<PostOut>(`/api/posts/${id}`, json("PATCH", body)),
  deleteDiaryEntry: (id: number) => request<void>(`/api/posts/${id}`, { method: "DELETE" }),
  reactToEntry: (id: number, emoji: string) =>
    request<PostOut>(`/api/posts/${id}/reactions`, json("POST", { emoji })),
  diaryFeed: (before?: number | null, limit = 8, club?: string) => {
    const query = new URLSearchParams({ limit: String(limit) });
    if (before) query.set("before", String(before));
    if (club) query.set("club", club);
    return request<DiaryFeedOut>(`/api/diary?${query}`);
  },

  clubs: () => request<ClubList>("/api/clubs"),
  setClubPick: (slug: string, body: ClubPickSetIn) =>
    request<ClubPickOut>(`${clubPath(slug)}/pick`, json("PUT", body)),
  patchClubPick: (slug: string, body: ClubPickPatchIn) =>
    request<ClubPickOut>(`${clubPath(slug)}/pick`, json("PATCH", body)),
  nominate: (slug: string, workId: number) =>
    request<VoteOut>(`${clubPath(slug)}/vote/nominations`, json("POST", { work_id: workId })),
};

/**
 * The single-club paths of the legacy app. The Rust server has none of them:
 * its club routes live under `/api/clubs/{slug}/…`. Only the club pages that
 * the club context (#48, part 2) rewrites still call these.
 */
export const legacyApi = {
  members: () => request<MemberOut[]>("/api/members"),
  clubPick: () => request<ClubPickCurrentOut>("/api/pick"),
  clubPickHistory: () => request<ClubPickHistoryOut>("/api/pick/history"),
  setClubPick: (body: ClubPickSetIn) => request<ClubPickOut>("/api/pick", json("PUT", body)),
  overlap: (includeReading = false) =>
    request<OverlapOut>(`/api/overlap${includeReading ? "?include_reading=true" : ""}`),
  nextUp: () => request<VoteOut>("/api/vote"),
  nominate: (workId: number) =>
    request<VoteOut>("/api/vote/nominations", json("POST", { work_id: workId })),
  castVote: (nominationId: number) =>
    request<VoteOut>("/api/vote/cast", json("POST", { nomination_id: nominationId })),
  applyWinner: (nominationId: number, meetingAt?: string | null) =>
    request<VoteApplyOut>(
      "/api/vote/apply",
      json("POST", { nomination_id: nominationId, meeting_at: meetingAt ?? null }),
    ),
};
