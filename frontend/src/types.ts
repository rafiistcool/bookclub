import type { Status } from "./constants";

export type ThemeId = "paper" | "slate" | "forest" | "ink";
export type ColorMode = "light" | "dark" | "system";

export type User = {
  id: number;
  username: string;
  theme: ThemeId;
  color_mode: ColorMode;
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
  rating: number | null;
  take: string;
  dnf_reason: string;
  progress: number | null;
};

export type SearchHit = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  on_shelf: Status | null;
  shelf_id: number | null;
  club_pick?: boolean;
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

export type BookReader = {
  username: string;
  status: Status;
  rating: number | null;
  progress: number | null;
};

export type BookDetail = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  description: string;
  subjects: string[];
  on_shelf: Status | null;
  shelf_id: number | null;
  club_pick: boolean;
  rating: number | null;
  take: string;
  dnf_reason: string;
  progress: number | null;
  readers: BookReader[];
  club_rating: number | null;
  rating_count: number;
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

export type ClubConfig = {
  name: string;
  theme: string;
  theme_dark: string;
  public_url: string;
  timezone: string;
};

export type ClubPickReader = {
  username: string;
  status: Status;
  rating: number | null;
  take: string;
  dnf_reason: string;
  progress: number | null;
};

export type ClubPick = {
  id: number;
  book: Book;
  set_by: string;
  note: string;
  started_at: string;
  ended_at: string | null;
  meeting_at: string | null;
  meeting_local: string | null;
  meeting_label: string | null;
  on_shelf: Status | null;
  shelf_id: number | null;
  readers: ClubPickReader[];
  reading: string[];
  finished: string[];
};

export type ClubPickBook = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  note?: string;
  meeting_at?: string | null;
};

export type ClubPickCurrent = {
  pick: ClubPick | null;
  timezone: string;
};

export type OverlapMember = {
  username: string;
  status: Status;
};

export type OverlapBook = {
  book: Book;
  count: number;
  members: OverlapMember[];
};

export type OverlapList = {
  items: OverlapBook[];
  include_reading: boolean;
};

export type Reaction = {
  emoji: string;
  count: number;
  mine: boolean;
  users: string[];
};

export type DiaryEntry = {
  id: number;
  author: string;
  mine: boolean;
  body: string;
  deleted: boolean;
  spoiler_upto: number | null;
  progress_at: number | null;
  status_at: Status | null;
  author_rating: number | null;
  parent_id: number | null;
  created_at: string;
  created_label: string;
  edited: boolean;
  reactions: Reaction[];
  replies: DiaryEntry[];
};

export type Diary = {
  ol_work_key: string;
  book_id: number | null;
  my_progress: number | null;
  my_status: Status | null;
  items: DiaryEntry[];
  timezone: string;
};

export type DiaryBookIn = {
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
};

export type DiaryEntryIn = {
  body: string;
  spoiler_upto?: number | null;
  parent_id?: number | null;
  book?: DiaryBookIn;
};

export type DiaryFeedItem = {
  entry: DiaryEntry;
  book: Book;
  parent_author: string | null;
  my_progress: number | null;
  my_status: Status | null;
};

export type DiaryFeed = {
  items: DiaryFeedItem[];
  has_more: boolean;
  timezone: string;
};

export type VoteBook = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
};

export type VoteNomination = {
  id: number;
  book: Book;
  nominated_by: string;
  votes: number;
  voters: string[];
  mine: boolean;
};

export type NextUpVote = {
  vote_id: number | null;
  nominations: VoteNomination[];
  my_vote_id: number | null;
  nomination_limit: number;
  can_nominate: boolean;
  timezone: string;
};

export type VoteApplyResult = {
  pick: ClubPick;
  vote: NextUpVote;
};

export type GoodreadsSkip = {
  title: string;
  reason: string;
};

export type GoodreadsImport = {
  imported: number;
  skipped: number;
  skips: GoodreadsSkip[];
};
