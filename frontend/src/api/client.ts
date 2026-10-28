import type {
  ActivityOut,
  AdminSummary,
  BookDetailOut,
  ClubIn,
  ClubList,
  ClubOut,
  ClubPatchIn,
  ClubStatsOut,
  CoverRequested,
  CoverSet,
  ClubPickCurrentOut,
  ClubPickHistoryOut,
  ClubPickOut,
  ClubPickPatchIn,
  ClubPickSetIn,
  CustomBookIn,
  DiaryEntryIn,
  DiaryFeedOut,
  DiaryOut,
  EditRequestCreated,
  EditRequestList,
  EditRequestOut,
  EditionList,
  EditionOut,
  FavoritesOut,
  ImportJobOut,
  InviteIn,
  InviteList,
  InviteOut,
  InvitePreview,
  LoginIn,
  MemberList,
  MemberOut,
  MilestoneIn,
  MilestoneListOut,
  MilestoneOut,
  NewEditionIn,
  NotificationsIn,
  NotificationsOut,
  OverlapOut,
  PersonalStatsOut,
  PickPostIn,
  PickThreadOut,
  PostOut,
  PostPatchIn,
  PreferencesIn,
  QuoteIn,
  QuoteListOut,
  QuoteOut,
  QuotePatchIn,
  RegisterIn,
  ReportCreated,
  ReportIn,
  ReportList,
  ReportOut,
  SearchHistoryOut,
  SearchPage,
  ServiceConfig,
  SessionList,
  ShelfAddIn,
  ShelfItemOut,
  ShelfListOut,
  ShelfPatchIn,
  SignupInviteIn,
  SpineColorsOut,
  TakeDownIn,
  UserOut,
  VerifyEmailPreview,
  VoteApplyOut,
  VoteOut,
  VoteSuggestionsOut,
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

/**
 * The fields of an edit request. The schema leaves `EditRequestIn.fields` an
 * untyped object; the keys and value types are those `apply_admin_fields`
 * accepts (text, a whole number, a list of subjects, or null to clear).
 */
export type EditFields = Record<string, string | number | string[] | null>;

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

  /** The link from the verification mail (`/verify-email#token=…`). Needs no session. */
  verifyEmail: (token: string) => request<void>("/api/auth/verify-email", json("POST", { token })),
  /** What that link would verify (the account and the address), without verifying it. */
  previewVerifyEmail: (token: string) =>
    request<VerifyEmailPreview>("/api/auth/verify-email/preview", json("POST", { token })),
  resendVerification: () => request<void>("/api/auth/verify-email/resend", { method: "POST" }),
  changeEmail: (email: string, currentPassword: string) =>
    request<void>("/api/auth/me/email", json("PUT", { email, current_password: currentPassword })),
  dismissEmailPrompt: () => request<void>("/api/auth/me/email-prompt/dismiss", { method: "POST" }),
  /** Always 204, whether or not the address belongs to an account. */
  requestPasswordReset: (email: string) =>
    request<void>("/api/auth/password-reset", json("POST", { email })),
  confirmPasswordReset: (token: string, newPassword: string) =>
    request<void>("/api/auth/password-reset/confirm", json("POST", { token, new_password: newPassword })),
  changePassword: (currentPassword: string, newPassword: string) =>
    request<void>(
      "/api/auth/password",
      json("PATCH", { current_password: currentPassword, new_password: newPassword }),
    ),
  sessions: () => request<SessionList>("/api/auth/sessions"),
  endSession: (id: number) => request<void>(`/api/auth/sessions/${id}`, { method: "DELETE" }),
  notifications: () => request<NotificationsOut>("/api/auth/notifications"),
  saveNotifications: (body: NotificationsIn) =>
    request<NotificationsOut>("/api/auth/notifications", json("PATCH", body)),
  deleteAccount: (password: string) => request<void>("/api/auth/me", json("DELETE", { password })),

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

  /** Quotes the viewer may see: one work's, one reader's, or both. */
  quotes: (params: { work?: number; username?: string; limit?: number } = {}) => {
    const query = new URLSearchParams();
    if (params.work) query.set("work", String(params.work));
    if (params.username) query.set("username", params.username);
    if (params.limit) query.set("limit", String(params.limit));
    const qs = query.toString();
    return request<QuoteListOut>(`/api/quotes${qs ? `?${qs}` : ""}`);
  },
  addQuote: (body: QuoteIn) => request<QuoteOut>("/api/quotes", json("POST", body)),
  editQuote: (id: number, body: QuotePatchIn) =>
    request<QuoteOut>(`/api/quotes/${id}`, json("PATCH", body)),
  deleteQuote: (id: number) => request<void>(`/api/quotes/${id}`, { method: "DELETE" }),

  diary: (workId: number | string) => request<DiaryOut>(`${workPath(workId)}/posts`),
  addDiaryEntry: (workId: number | string, body: DiaryEntryIn) =>
    request<PostOut>(`${workPath(workId)}/posts`, json("POST", body)),
  editDiaryEntry: (id: number, body: PostPatchIn) =>
    request<PostOut>(`/api/posts/${id}`, json("PATCH", body)),
  deleteDiaryEntry: (id: number) => request<void>(`/api/posts/${id}`, { method: "DELETE" }),
  reactToEntry: (id: number, emoji: string) =>
    request<PostOut>(`/api/posts/${id}/reactions`, json("POST", { emoji })),
  /** `mine`: only the caller's own diary writing (`/diary`). */
  diaryFeed: (before?: number | null, limit = 8, club?: string, mine = false) => {
    const query = new URLSearchParams({ limit: String(limit) });
    if (before) query.set("before", String(before));
    if (club) query.set("club", club);
    if (mine) query.set("mine", "true");
    return request<DiaryFeedOut>(`/api/diary?${query}`);
  },

  /** The home feed (every club of the viewer and their own events), or one club's feed. */
  activity: (params: { before?: number | null; limit?: number; club?: string } = {}) => {
    const query = new URLSearchParams({ limit: String(params.limit ?? 20) });
    if (params.before) query.set("before", String(params.before));
    const base = params.club ? `${clubPath(params.club)}/activity` : "/api/activity";
    return request<ActivityOut>(`${base}?${query}`);
  },

  clubs: () => request<ClubList>("/api/clubs"),
  club: (slug: string) => request<ClubOut>(clubPath(slug)),
  createClub: (body: ClubIn) => request<ClubOut>("/api/clubs", json("POST", body)),
  updateClub: (slug: string, body: ClubPatchIn) =>
    request<ClubOut>(clubPath(slug), json("PATCH", body)),
  deleteClub: (slug: string) => request<void>(clubPath(slug), { method: "DELETE" }),
  clubMembers: (slug: string) => request<MemberList>(`${clubPath(slug)}/members`),
  changeMemberRole: (slug: string, username: string, role: "admin" | "member") =>
    request<MemberOut>(`${clubPath(slug)}/members/${enc(username)}`, json("PATCH", { role })),
  removeMember: (slug: string, username: string) =>
    request<void>(`${clubPath(slug)}/members/${enc(username)}`, { method: "DELETE" }),
  leaveClub: (slug: string) => request<void>(`${clubPath(slug)}/leave`, { method: "POST" }),
  transferClub: (slug: string, username: string) =>
    request<ClubOut>(`${clubPath(slug)}/transfer`, json("POST", { username })),
  clubInvites: (slug: string) => request<InviteList>(`${clubPath(slug)}/invites`),
  createInvite: (slug: string, body: InviteIn) =>
    request<InviteOut>(`${clubPath(slug)}/invites`, json("POST", body)),
  revokeInvite: (slug: string, id: number) =>
    request<void>(`${clubPath(slug)}/invites/${id}`, { method: "DELETE" }),
  /** Public: which club an invite code leads to. */
  previewInvite: (code: string) => request<InvitePreview>(`/api/invites/${enc(code)}`),
  acceptInvite: (code: string) =>
    request<ClubOut>(`/api/invites/${enc(code)}/accept`, { method: "POST" }),

  clubPick: (slug: string) => request<ClubPickCurrentOut>(`${clubPath(slug)}/pick`),
  clubPickHistory: (slug: string) => request<ClubPickHistoryOut>(`${clubPath(slug)}/pick/history`),
  setClubPick: (slug: string, body: ClubPickSetIn) =>
    request<ClubPickOut>(`${clubPath(slug)}/pick`, json("PUT", body)),
  /** The meeting edit: only the keys present change (`meeting_at: null` clears it). */
  patchClubPick: (slug: string, body: ClubPickPatchIn) =>
    request<ClubPickOut>(`${clubPath(slug)}/pick`, json("PATCH", body)),
  /** The calendar file of the current meeting, as a same-origin download link. */
  meetingIcsPath: (slug: string) => `${clubPath(slug)}/pick/meeting.ics`,
  milestones: (slug: string, pickId: number) =>
    request<MilestoneListOut>(`${clubPath(slug)}/picks/${pickId}/milestones`),
  addMilestone: (slug: string, pickId: number, body: MilestoneIn) =>
    request<MilestoneOut>(`${clubPath(slug)}/picks/${pickId}/milestones`, json("POST", body)),
  deleteMilestone: (slug: string, pickId: number, id: number) =>
    request<void>(`${clubPath(slug)}/picks/${pickId}/milestones/${id}`, { method: "DELETE" }),
  /** The discussion of the current pick, or of an earlier one by id. */
  pickThread: (slug: string, pickId?: number | null) =>
    request<PickThreadOut>(
      pickId ? `${clubPath(slug)}/picks/${pickId}/posts` : `${clubPath(slug)}/pick/posts`,
    ),
  addPickPost: (slug: string, pickId: number | null | undefined, body: PickPostIn) =>
    request<PostOut>(
      pickId ? `${clubPath(slug)}/picks/${pickId}/posts` : `${clubPath(slug)}/pick/posts`,
      json("POST", body),
    ),

  vote: (slug: string) => request<VoteOut>(`${clubPath(slug)}/vote`),
  nominate: (slug: string, workId: number) =>
    request<VoteOut>(`${clubPath(slug)}/vote/nominations`, json("POST", { work_id: workId })),
  castVote: (slug: string, nominationId: number) =>
    request<VoteOut>(`${clubPath(slug)}/vote/cast`, json("POST", { nomination_id: nominationId })),
  applyVote: (slug: string, nominationId: number, meetingAt?: string | null) =>
    request<VoteApplyOut>(
      `${clubPath(slug)}/vote/apply`,
      json("POST", { nomination_id: nominationId, meeting_at: meetingAt ?? null }),
    ),
  setVoteDeadline: (slug: string, closesAt: string | null) =>
    request<VoteOut>(`${clubPath(slug)}/vote`, json("PATCH", { closes_at: closesAt })),
  voteSuggestions: (slug: string) =>
    request<VoteSuggestionsOut>(`${clubPath(slug)}/vote/suggestions`),
  overlap: (slug: string, includeReading = false) =>
    request<OverlapOut>(
      `${clubPath(slug)}/overlap${includeReading ? "?include_reading=true" : ""}`,
    ),

  /**
   * A photo of the reader's copy as the cover: 200 `CoverSet` when it became
   * the cover, 202 `CoverRequested` when a cover existed and an admin decides.
   */
  uploadCover: (workId: number, file: File, editionId?: number | null) => {
    const body = upload(file);
    if (editionId != null) body.append("edition_id", String(editionId));
    return request<CoverSet | CoverRequested>(`${workPath(workId)}/cover`, { method: "POST", body });
  },
  /** "These are the same book": `workId` should be merged into `intoWorkId` (an admin decides). */
  requestMerge: (workId: number, intoWorkId: number, note: string) =>
    request<EditRequestCreated>(
      `${workPath(workId)}/merge-requests`,
      json("POST", { into_work_id: intoWorkId, ...(note ? { note } : {}) }),
    ),
  /** Every edition of a work, oldest first. */
  editions: (workId: number) => request<EditionList>(`${workPath(workId)}/editions`),
  /** One edition. */
  edition: (editionId: number) => request<EditionOut>(`/api/books/editions/${editionId}`),
  /** A missing edition (201). A known ISBN is 409 with `work_id` and `edition_id` in the body. */
  addEdition: (workId: number, body: NewEditionIn) =>
    request<EditionOut>(`${workPath(workId)}/editions`, json("POST", body)),
  /** A correction of a book's facts, for the admin queue. Only the changed fields; null clears one. */
  requestWorkEdit: (workId: number, fields: EditFields, note: string) =>
    request<EditRequestCreated>(
      `${workPath(workId)}/edit-requests`,
      json("POST", { fields, ...(note ? { note } : {}) }),
    ),
  /** The same for one edition. */
  requestEditionEdit: (editionId: number, fields: EditFields, note: string) =>
    request<EditRequestCreated>(
      `/api/books/editions/${editionId}/edit-requests`,
      json("POST", { fields, ...(note ? { note } : {}) }),
    ),

  // Instance admins only; everyone else gets 404 `Not found`.
  /** The open catalog requests (edits, cover photos, merges), oldest first. */
  adminEditRequests: () => request<EditRequestList>("/api/admin/edit-requests"),
  approveEditRequest: (id: number) =>
    request<EditRequestOut>(`/api/admin/edit-requests/${id}/approve`, { method: "POST" }),
  rejectEditRequest: (id: number, note: string) =>
    request<EditRequestOut>(
      `/api/admin/edit-requests/${id}/reject`,
      json("POST", note ? { note } : {}),
    ),
  /** The open cover reports, oldest first. */
  adminReports: () => request<ReportList>("/api/admin/reports"),
  /** `remove` takes the picture down everywhere; `dismiss` keeps it. */
  resolveReport: (id: number, action: "remove" | "dismiss") =>
    request<ReportOut>(`/api/admin/reports/${id}/resolve`, json("POST", { action })),
  /** Both queues at a glance: open and overdue counts, the oldest item, 90-day report statistics. */
  adminSummary: () => request<AdminSummary>("/api/admin/summary"),
  /** A page of the report queue: `open` oldest first, `resolved` newest decision first; 50 a page. */
  adminReportPage: (status: "open" | "resolved", page = 1) =>
    request<ReportList>(`/api/admin/reports?status=${status}&page=${page}`),
  /** A page of the catalog queue, the same way. */
  adminEditRequestPage: (status: "open" | "resolved", page = 1) =>
    request<EditRequestList>(`/api/admin/edit-requests?status=${status}&page=${page}`),
  /**
   * Takes a picture down directly. With `notice` (an emailed notice), the
   * takedown is recorded as a resolved report that arrived at `received_at`.
   */
  adminTakeDown: (publicId: string, notice?: TakeDownIn) =>
    request<void>(`/api/admin/media/${enc(publicId)}`, notice ? json("DELETE", notice) : { method: "DELETE" }),
  /** Sets a book's facts at once (provenance `admin`: no catalog source overwrites them). Null clears a field. */
  adminEditWork: (workId: number | string, fields: EditFields) =>
    request<BookDetailOut>(`/api/admin/works/${enc(String(workId))}`, json("PATCH", { fields })),
  /** The same for one edition. */
  adminEditEdition: (editionId: number, fields: EditFields) =>
    request<EditionOut>(`/api/admin/editions/${editionId}`, json("PATCH", { fields })),
  /** Club-less sign-up invites, newest first. */
  adminInvites: () => request<InviteList>("/api/admin/invites"),
  createSignupInvite: (body: SignupInviteIn) =>
    request<InviteOut>("/api/admin/invites", json("POST", body)),

  /** A reader's year in books: the caller's without `username`; the newest year without `year`. */
  personalStats: (params: { username?: string; year?: number } = {}) => {
    const query = new URLSearchParams();
    if (params.username) query.set("username", params.username);
    if (params.year) query.set("year", String(params.year));
    const qs = query.toString();
    return request<PersonalStatsOut>(`/api/stats${qs ? `?${qs}` : ""}`);
  },
  clubStats: (slug: string, year?: number) =>
    request<ClubStatsOut>(`${clubPath(slug)}/stats${year ? `?year=${year}` : ""}`),
};
