// Complete API objects for tests, so each test only spells out what it is about.
import type {
  BookDetail,
  Book as BookOut,
  DiaryEntry,
  Favorite,
  GoodreadsImport,
  SearchHit,
  ShelfItem,
  User,
} from "../types";

export function bookOut(overrides: Partial<BookOut> = {}): BookOut {
  const id = overrides.id ?? 1;
  return {
    id,
    ol_work_key: `/works/OL${id}W`,
    title: `Book ${id}`,
    authors: "Author",
    year: null,
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    pages: null,
    spine_color: null,
    source: "openlibrary",
    custom: false,
    ...overrides,
  };
}

export function shelfItem(
  overrides: Partial<Omit<ShelfItem, "book">> & { book?: Partial<BookOut> } = {},
): ShelfItem {
  const { book, ...rest } = overrides;
  const id = rest.id ?? 1;
  return {
    id,
    status: "want_to_read",
    position: 0,
    shelf_position: null,
    updated_at: "2026-01-01T00:00:00Z",
    started_at: null,
    finished_at: null,
    edition: null,
    rating: null,
    rating_visibility: "audience",
    take: "",
    dnf_reason: "",
    progress: null,
    ...rest,
    book: bookOut({ id, ...book }),
  };
}

export function favorite(position: number, book: Partial<BookOut> = {}): Favorite {
  return { position, book: bookOut({ id: position, ...book }) };
}

export function searchHit(overrides: Partial<SearchHit> = {}): SearchHit {
  const id = overrides.id ?? 1;
  return {
    id,
    ol_work_key: `/works/OL${id}W`,
    title: "Circe",
    authors: "Madeline Miller",
    year: 2018,
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    isbn: null,
    source: "openlibrary",
    custom: false,
    on_shelf: false,
    shelf_id: null,
    club_picks: [],
    planned_publication_date: null,
    ...overrides,
  };
}

export function bookDetail(overrides: Partial<BookDetail> = {}): BookDetail {
  const id = overrides.id ?? 1;
  return {
    id,
    ol_work_key: `/works/OL${id}W`,
    title: "Circe",
    subtitle: "",
    authors: "Madeline Miller",
    year: 2018,
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    description: "",
    subjects: [],
    pages: null,
    ol_rating: null,
    ol_rating_count: null,
    source: "openlibrary",
    custom: false,
    editions_count: 0,
    on_shelf: null,
    shelf_id: null,
    edition_id: null,
    rating: null,
    rating_visibility: null,
    take: "",
    dnf_reason: "",
    progress: null,
    readers: [],
    club_rating: null,
    rating_count: 0,
    quote_count: 0,
    favorite_position: null,
    club_picks: [],
    can_refresh: true,
    ...overrides,
  };
}

export function user(overrides: Partial<User> = {}): User {
  return {
    id: 1,
    username: "ada",
    email: "ada@example.com",
    email_verified: true,
    pending_email: null,
    needs_email_prompt: false,
    theme: "paper",
    color_mode: "system",
    locale: "en",
    timezone: "UTC",
    avatar_url: null,
    is_admin: false,
    clubs: [],
    ...overrides,
  };
}

export function post(overrides: Partial<DiaryEntry> = {}): DiaryEntry {
  return {
    id: 1,
    author: "ada",
    author_avatar_url: null,
    mine: false,
    body: "",
    deleted: false,
    visibility: "audience",
    spoiler_upto: null,
    progress_at: null,
    status_at: null,
    author_rating: null,
    parent_id: null,
    club: null,
    pick_id: null,
    milestone_id: null,
    created_at: "2026-01-01T00:00:00Z",
    created_label: "",
    edited: false,
    reactions: [],
    replies: [],
    ...overrides,
  };
}

export function importJob(overrides: Partial<GoodreadsImport> = {}): GoodreadsImport {
  return {
    id: 7,
    status: "queued",
    total: 0,
    processed: 0,
    imported: 0,
    skipped: 0,
    skips: [],
    error: null,
    ...overrides,
  };
}
