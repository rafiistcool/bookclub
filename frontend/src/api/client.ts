import type { Invite, Member, SearchPage, SearchParams, ShelfItem, ShelfList, User } from "../types";
import type { Status } from "../constants";

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
  if (init.body && !headers.has("Content-Type")) {
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
  me: () => request<User>("/api/auth/me"),
  register: (body: { username: string; password: string; invite_code: string }) =>
    request<User>("/api/auth/register", { method: "POST", body: JSON.stringify(body) }),
  login: (body: { username: string; password: string }) =>
    request<void>("/api/auth/login", { method: "POST", body: JSON.stringify(body) }),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),
  search: (params: SearchParams = {}) => {
    const query = new URLSearchParams();
    if (params.q) query.set("q", params.q);
    if (params.subject) query.set("subject", params.subject);
    if (params.sort) query.set("sort", params.sort);
    if (params.page) query.set("page", String(params.page));
    if (params.limit) query.set("limit", String(params.limit));
    const qs = query.toString();
    return request<SearchPage>(`/api/books/search${qs ? `?${qs}` : ""}`);
  },
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
  }) => request<ShelfItem>("/api/shelf", { method: "POST", body: JSON.stringify(body) }),
  patchShelf: (id: number, body: { status?: Status; position?: number }) =>
    request<ShelfItem>(`/api/shelf/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  removeFromShelf: (id: number) =>
    request<void>(`/api/shelf/${id}`, { method: "DELETE" }),
  members: () => request<Member[]>("/api/members"),
  invites: () => request<Invite[]>("/api/invites"),
  createInvite: () =>
    request<{ code: string }>("/api/invites", { method: "POST" }),
};
