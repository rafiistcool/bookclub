import type { FinishNote, Status } from "../constants";
import type {
  ActivityPage,
  BookDetails,
  BookRef,
  ClubConfig,
  ClubPick,
  ClubPickBook,
  ClubPickCurrent,
  GoodreadsImport,
  Invite,
  IsbnHit,
  Member,
  Milestone,
  MilestoneInput,
  MilestoneList,
  NextUpVote,
  NotificationPrefs,
  OverlapList,
  PickPost,
  PickThread,
  PushSubscriptionRow,
  Quote,
  SearchPage,
  SearchParams,
  ShelfItem,
  ShelfList,
  Stats,
  User,
  VoteApplyResult,
  VoteSuggestion,
} from "../types";

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

const json = (body: unknown): RequestInit["body"] => JSON.stringify(body);

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "" || value === false) continue;
    query.set(key, String(value));
  }
  const text = query.toString();
  return text ? `?${text}` : "";
}

export const api = {
  // --- config / auth ------------------------------------------------------
  config: () => request<ClubConfig>("/api/config"),
  me: () => request<User>("/api/auth/me"),
  register: (body: { username: string; password: string; invite_code: string }) =>
    request<User>("/api/auth/register", { method: "POST", body: json(body) }),
  login: (body: { username: string; password: string }) =>
    request<void>("/api/auth/login", { method: "POST", body: json(body) }),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),
  changePassword: (body: { current_password: string; new_password: string }) =>
    request<void>("/api/auth/password", { method: "PATCH", body: json(body) }),
  notificationPrefs: () => request<NotificationPrefs>("/api/auth/notifications"),
  updateNotificationPrefs: (body: Partial<NotificationPrefs>) =>
    request<NotificationPrefs>("/api/auth/notifications", { method: "PATCH", body: json(body) }),

  // --- books -----------------------------------------------------------------
  search: (params: SearchParams = {}) =>
    request<SearchPage>(`/api/books/search${qs(params)}`),
  bookDetails: (olWorkKey: string) =>
    request<BookDetails>(`/api/books/work/${encodeURIComponent(olWorkKey.replace("/works/", ""))}`),
  isbnLookup: (isbn: string) => request<IsbnHit>(`/api/books/isbn/${encodeURIComponent(isbn)}`),

  // --- shelf -----------------------------------------------------------------
  myShelf: () => request<ShelfList>("/api/shelf"),
  friendShelf: (username: string) =>
    request<ShelfList>(`/api/shelf?username=${encodeURIComponent(username)}`),
  addToShelf: (body: BookRef & { status: Status } & FinishNote) =>
    request<ShelfItem>("/api/shelf", { method: "POST", body: json(body) }),
  patchShelf: (id: number, body: { status?: Status; position?: number } & FinishNote) =>
    request<ShelfItem>(`/api/shelf/${id}`, { method: "PATCH", body: json(body) }),
  removeFromShelf: (id: number) => request<void>(`/api/shelf/${id}`, { method: "DELETE" }),
  importGoodreads: (file: File) => {
    const body = new FormData();
    body.append("file", file);
    return request<GoodreadsImport>("/api/shelf/import", { method: "POST", body });
  },

  // --- members / invites ---------------------------------------------------------
  members: () => request<Member[]>("/api/members"),
  invites: () => request<Invite[]>("/api/invites"),
  createInvite: () => request<{ code: string }>("/api/invites", { method: "POST" }),

  // --- club pick -----------------------------------------------------------------
  clubPick: () => request<ClubPickCurrent>("/api/pick"),
  clubPickHistory: () => request<{ items: ClubPick[]; timezone: string }>("/api/pick/history"),
  setClubPick: (body: ClubPickBook) =>
    request<ClubPick>("/api/pick", { method: "PUT", body: json(body) }),
  clearClubPick: () => request<ClubPickCurrent>("/api/pick", { method: "DELETE" }),
  meetingIcsUrl: () => "/api/pick/meeting.ics",

  // --- notes ------------------------------------------------------------------------
  pickPosts: (pickId?: number) =>
    request<PickThread>(pickId ? `/api/pick/${pickId}/posts` : "/api/pick/posts"),
  addPickPost: (
    body: { body: string; spoiler_upto?: number | null; milestone_id?: number | null },
    pickId?: number,
  ) =>
    request<PickPost>(pickId ? `/api/pick/${pickId}/posts` : "/api/pick/posts", {
      method: "POST",
      body: json(body),
    }),
  editPickPost: (postId: number, body: { body?: string; spoiler_upto?: number | null }) =>
    request<PickPost>(`/api/pick/posts/${postId}`, { method: "PATCH", body: json(body) }),
  deletePickPost: (postId: number) =>
    request<void>(`/api/pick/posts/${postId}`, { method: "DELETE" }),
  toggleReaction: (postId: number, emoji: string) =>
    request<PickPost>(`/api/pick/posts/${postId}/reactions`, {
      method: "POST",
      body: json({ emoji }),
    }),

  // --- milestones -----------------------------------------------------------------------
  milestones: (pickId: number) => request<MilestoneList>(`/api/pick/${pickId}/milestones`),
  addMilestone: (pickId: number, body: MilestoneInput) =>
    request<Milestone>(`/api/pick/${pickId}/milestones`, { method: "POST", body: json(body) }),
  editMilestone: (pickId: number, milestoneId: number, body: MilestoneInput) =>
    request<Milestone>(`/api/pick/${pickId}/milestones/${milestoneId}`, {
      method: "PATCH",
      body: json(body),
    }),
  deleteMilestone: (pickId: number, milestoneId: number) =>
    request<void>(`/api/pick/${pickId}/milestones/${milestoneId}`, { method: "DELETE" }),

  // --- overlap / vote -------------------------------------------------------------------------
  overlap: (includeReading = false) =>
    request<OverlapList>(`/api/overlap${includeReading ? "?include_reading=true" : ""}`),
  nextUp: () => request<NextUpVote>("/api/vote"),
  setVoteDeadline: (closesAt: string | null) =>
    request<NextUpVote>("/api/vote", { method: "PATCH", body: json({ closes_at: closesAt }) }),
  nominate: (body: BookRef) =>
    request<NextUpVote>("/api/vote/nominations", { method: "POST", body: json(body) }),
  castVote: (nominationId: number) =>
    request<NextUpVote>("/api/vote/cast", {
      method: "POST",
      body: json({ nomination_id: nominationId }),
    }),
  applyWinner: (nominationId: number, meetingAt?: string | null) =>
    request<VoteApplyResult>("/api/vote/apply", {
      method: "POST",
      body: json({ nomination_id: nominationId, meeting_at: meetingAt ?? null }),
    }),
  voteSuggestions: () => request<{ items: VoteSuggestion[] }>("/api/vote/suggestions"),

  // --- activity / stats / quotes ------------------------------------------------------------------
  activity: (params: { before?: number; limit?: number; username?: string } = {}) =>
    request<ActivityPage>(`/api/activity${qs(params)}`),
  stats: (year?: number) => request<Stats>(`/api/stats${qs({ year })}`),
  quotes: (params: { work?: string; username?: string; limit?: number } = {}) =>
    request<{ items: Quote[] }>(`/api/quotes${qs(params)}`),
  addQuote: (body: BookRef & { body: string; page?: number | null }) =>
    request<Quote>("/api/quotes", { method: "POST", body: json(body) }),
  editQuote: (quoteId: number, body: { body?: string; page?: number | null }) =>
    request<Quote>(`/api/quotes/${quoteId}`, { method: "PATCH", body: json(body) }),
  deleteQuote: (quoteId: number) => request<void>(`/api/quotes/${quoteId}`, { method: "DELETE" }),

  // --- push ---------------------------------------------------------------------------------------------
  vapidPublicKey: () => request<{ public_key: string }>("/api/push/vapid"),
  pushSubscriptions: () => request<{ items: PushSubscriptionRow[] }>("/api/push/subscriptions"),
  subscribePush: (body: { endpoint: string; keys: { p256dh: string; auth: string }; user_agent?: string }) =>
    request<PushSubscriptionRow>("/api/push/subscriptions", { method: "POST", body: json(body) }),
  unsubscribePush: (endpoint: string) =>
    request<void>("/api/push/subscriptions", { method: "DELETE", body: json({ endpoint }) }),
  testPush: () => request<{ sent_to: number }>("/api/push/test", { method: "POST" }),
};

export type Api = typeof api;
