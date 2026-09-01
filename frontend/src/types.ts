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
  pages?: number | null;
};

/** The minimum needed to add or nominate a book from anywhere. */
export type BookRef = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
};

export type ShelfItem = {
  id: number;
  status: Status;
  position: number;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
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

export type BookMember = {
  username: string;
  status: Status;
  rating: number | null;
  take: string;
  progress: number | null;
  finished_at: string | null;
};

export type BookDetails = {
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  cover_url: string | null;
  description: string;
  pages: number | null;
  subjects: string[];
  ol_rating: number | null;
  ol_rating_count: number | null;
  on_shelf: Status | null;
  shelf_id: number | null;
  club_pick: boolean;
  members: BookMember[];
  quote_count: number;
};

export type IsbnHit = {
  isbn: string;
  ol_work_key: string;
  title: string;
  authors: string;
  cover_id: number | null;
  year: number | null;
  pages: number | null;
  on_shelf: Status | null;
  shelf_id: number | null;
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

export type ClubPickBook = BookRef & {
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

export type PickPost = {
  id: number;
  author: string;
  mine: boolean;
  body: string;
  spoiler_upto: number | null;
  milestone_id: number | null;
  created_at: string;
  created_label: string;
  edited: boolean;
  reactions: Reaction[];
};

export type PickThread = {
  pick_id: number;
  can_post: boolean;
  my_progress: number | null;
  my_status: Status | null;
  items: PickPost[];
  timezone: string;
};

export type Milestone = {
  id: number;
  pick_id: number;
  title: string;
  chapter_from: number | null;
  chapter_to: number | null;
  due_at: string | null;
  due_local: string | null;
  due_label: string | null;
  position: number;
  note_count: number;
  passed: boolean;
};

export type MilestoneList = {
  pick_id: number;
  items: Milestone[];
  timezone: string;
};

export type MilestoneInput = {
  title: string;
  chapter_from?: number | null;
  chapter_to?: number | null;
  due_at?: string | null;
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
  closes_at: string | null;
  closes_local: string | null;
  closes_label: string | null;
  not_voted: string[];
  voted_count: number;
  member_count: number;
  leader_id: number | null;
};

export type VoteApplyResult = {
  pick: ClubPick;
  vote: NextUpVote;
};

export type VoteSuggestion = {
  book: Book;
  score: number;
  reasons: string[];
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

export type ActivityItem = {
  id: number;
  kind: string;
  actor: string;
  mine: boolean;
  book: Book | null;
  pick_id: number | null;
  payload: Record<string, unknown>;
  created_at: string;
  created_label: string;
};

export type ActivityPage = {
  items: ActivityItem[];
  has_more: boolean;
  timezone: string;
};

export type MemberYear = {
  username: string;
  finished: number;
  dnf: number;
  pages: number;
  average_rating: number | null;
  five_stars: number;
  top_book: Book | null;
  longest_book: Book | null;
  quotes: number;
};

export type PickStat = {
  pick_id: number;
  book: Book;
  set_by: string;
  started_at: string;
  ended_at: string | null;
  readers: number;
  finished: number;
  dnf: number;
  average_rating: number | null;
  ratings: number[];
};

export type Stats = {
  year: number;
  years: number[];
  members: MemberYear[];
  by_month: { month: string; finished: number }[];
  picks: PickStat[];
  club_finished: number;
  club_pages: number;
  club_average_rating: number | null;
  best_pick: PickStat | null;
};

export type Quote = {
  id: number;
  book: Book;
  author: string;
  mine: boolean;
  body: string;
  page: number | null;
  created_at: string;
  created_label: string;
};

export type NotificationPrefs = {
  notify_meeting: boolean;
  notify_pick: boolean;
  notify_note: boolean;
};

export type PushSubscriptionRow = {
  id: number;
  endpoint_tail: string;
  user_agent: string;
  created_at: string;
};
