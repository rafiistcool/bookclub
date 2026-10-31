// Complete API objects for tests, so each test only spells out what it is about.
import type { EditionOut } from "../api/generated";
import type { IsbnHitOut } from "../api/generated";
import type {
  ActivityItem,
  BookDetail,
  Book as BookOut,
  ClubOut,
  ClubPick,
  DiaryEntry,
  Invite,
  Member,
  NextUpVote,
  VoteNomination,
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

export function edition(overrides: Partial<EditionOut> = {}): EditionOut {
  return {
    id: 1,
    work_id: 9,
    isbn13: null,
    isbn10: null,
    title: null,
    subtitle: null,
    publisher: "",
    language: null,
    format: null,
    pages: null,
    publication_year: null,
    planned_publication_date: null,
    contributors: [],
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    source: "dnb",
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
    needs_terms: false,
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

export function clubOut(overrides: Partial<ClubOut> = {}): ClubOut {
  return {
    slug: "readers-ab12",
    name: "Readers",
    timezone: "Europe/Berlin",
    role: "member",
    member_count: 3,
    created_at: "2026-01-01T00:00:00Z",
    current_pick: null,
    ...overrides,
  };
}

export function member(overrides: Partial<Member> = {}): Member {
  return {
    username: "ada",
    role: "member",
    joined_at: "2026-01-01T00:00:00Z",
    you: false,
    avatar_url: null,
    currently_reading_count: 0,
    currently_reading_preview: [],
    ...overrides,
  };
}

export function invite(overrides: Partial<Invite> = {}): Invite {
  const code = overrides.code ?? "7K3M9QX2HD";
  return {
    id: 1,
    code,
    url: `http://localhost/join/${code}`,
    created_by: "ada",
    created_at: "2026-01-01T00:00:00Z",
    expires_at: "2026-01-31T00:00:00Z",
    max_uses: 1,
    use_count: 0,
    used_by: [],
    revoked: false,
    active: true,
    personal: false,
    invitee: null,
    ...overrides,
  };
}

export function clubPick(overrides: Partial<ClubPick> = {}): ClubPick {
  return {
    id: 1,
    book: bookOut(),
    set_by: "ada",
    note: "",
    started_at: "2026-01-01T00:00:00Z",
    ended_at: null,
    meeting_at: null,
    meeting_local: null,
    meeting_label: null,
    on_shelf: null,
    shelf_id: null,
    readers: [],
    reading: [],
    finished: [],
    ...overrides,
  };
}

export function nomination(overrides: Partial<VoteNomination> = {}): VoteNomination {
  const id = overrides.id ?? 1;
  return {
    id,
    book: bookOut({ id }),
    nominated_by: "ada",
    votes: 0,
    voters: [],
    mine: false,
    ...overrides,
  };
}

export function vote(overrides: Partial<NextUpVote> = {}): NextUpVote {
  return {
    vote_id: null,
    nominations: [],
    my_vote_id: null,
    nomination_limit: 6,
    can_nominate: true,
    timezone: "Europe/Berlin",
    closes_at: null,
    closes_local: null,
    closes_label: null,
    not_voted: [],
    voted_count: 0,
    member_count: 3,
    leader_id: null,
    ...overrides,
  };
}

export function activityItem(
  overrides: Omit<Partial<ActivityItem>, "payload"> & { payload?: Record<string, unknown> } = {},
): ActivityItem {
  const { payload, ...rest } = overrides;
  return {
    id: 1,
    kind: "note_posted",
    actor: "ada",
    mine: false,
    book: bookOut(),
    pick_id: null,
    club: null,
    created_at: "2026-01-01T00:00:00Z",
    created_label: "Jan 1",
    ...rest,
    payload: (payload ?? {}) as ActivityItem["payload"],
  };
}

/** `GET /api/books/isbn/{isbn}`: a catalog edition, not on the viewer's shelf, without a cover. */
export function isbnHit(overrides: Partial<IsbnHitOut> = {}): IsbnHitOut {
  return {
    isbn: "9780316769488",
    work_id: 9,
    edition_id: 40,
    ol_work_key: "/works/OL9W",
    title: "The Catcher in the Rye",
    authors: "J. D. Salinger",
    year: 1951,
    pages: 277,
    cover_url: null,
    cover_thumb_url: null,
    cover_public_id: null,
    on_shelf: false,
    shelf_id: null,
    ...overrides,
  };
}
