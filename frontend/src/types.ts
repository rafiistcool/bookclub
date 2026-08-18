import type { Status } from "./constants";

export type User = {
  id: number;
  username: string;
};

export type Book = {
  id: number;
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  cover_url: string | null;
};

export type ShelfItem = {
  id: number;
  status: Status;
  position: number;
  updated_at: string;
  book: Book;
};

export type SearchHit = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  on_shelf: Status | null;
  shelf_id: number | null;
};

export type SearchSort = "readinglog" | "new" | "title" | "relevance";

export type SearchParams = {
  q?: string;
  subject?: string;
  sort?: SearchSort;
  page?: number;
  limit?: number;
};

export type SearchPage = {
  items: SearchHit[];
  page: number;
  has_more: boolean;
};

export type ShelfList = {
  user: User;
  items: ShelfItem[];
};

export type Member = {
  username: string;
  currently_reading_count: number;
  currently_reading_preview: { title: string; cover_id: number | null }[];
};

export type Invite = {
  code: string;
  used: boolean;
  used_by: string | null;
  created_at: string;
};
