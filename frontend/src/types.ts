// The API's shapes come from the server's OpenAPI document (src/api/generated.ts,
// written only by `npm run gen:api`). The names below are the app's names for
// them; new code may import from "./api/generated" directly. Only types the
// server does not describe (client-side unions, request helpers) are defined here.
import type {
  BookDetailOut,
  BookOut,
  ClubPickCurrentOut,
  ClubPickOut,
  DiaryFeedItemOut,
  DiaryFeedOut,
  DiaryOut,
  FavoriteOut,
  FavoritesOut,
  ImportJobOut,
  ImportSkipOut,
  InviteOut,
  MemberOut,
  OverlapBookOut,
  OverlapOut,
  PostOut,
  ReactionOut,
  SearchHistoryOut,
  ServiceConfig,
  ShelfItemOut,
  ShelfListOut,
  UserOut,
  VoteApplyOut,
  VoteNominationOut,
  VoteOut,
} from "./api/generated";

export type {
  ActivityItem,
  ActivityOut,
  BookReader,
  ClubOut,
  ClubPickReader,
  ClubPickSummary,
  ClubRef,
  ClubRole,
  ClubSummary,
  InvitePreview,
  MilestoneOut,
  MilestoneUnit,
  PickThreadOut,
  QuoteOut,
  VoteReasonOut,
  VoteSuggestionOut,
  CustomBookIn,
  DiaryEntryIn,
  EditionOut,
  ImportSkipCode,
  ImportStatus,
  OverlapMember,
  SearchHit,
  SearchPage,
  ShelfStatus,
  UserSummary,
  Visibility,
} from "./api/generated";

export type ThemeId = "paper" | "slate" | "forest" | "ink";
export type ColorMode = "light" | "dark" | "system";
export type Locale = "en" | "de";

/** The signed-in user (`GET /api/auth/me`). */
export type User = UserOut;
/** A work as every list shows it; edition-aware where the server has an edition context. */
export type Book = BookOut;
export type BookDetail = BookDetailOut;
export type ShelfItem = ShelfItemOut;
export type ShelfList = ShelfListOut;
export type Favorite = FavoriteOut;
export type FavoritesList = FavoritesOut;
export type SearchHistoryEntry = SearchHistoryOut;
/** Service branding from `GET /api/config` (its legal fields are the service store's). */
export type ServiceBrand = Pick<ServiceConfig, "name" | "theme" | "theme_dark" | "public_url" | "signup">;

export type GoodreadsImport = ImportJobOut;
export type GoodreadsSkip = ImportSkipOut;

export type Member = MemberOut;
export type Invite = InviteOut;
export type ClubPick = ClubPickOut;
export type ClubPickCurrent = ClubPickCurrentOut;
export type OverlapBook = OverlapBookOut;
export type OverlapList = OverlapOut;
export type Reaction = ReactionOut;
export type DiaryEntry = PostOut;
export type Diary = DiaryOut;
export type DiaryFeedItem = DiaryFeedItemOut;
export type DiaryFeed = DiaryFeedOut;
export type VoteNomination = VoteNominationOut;
export type NextUpVote = VoteOut;
export type VoteApplyResult = VoteApplyOut;

export type SearchSort = "readinglog" | "new" | "title" | "relevance";

export type SearchParams = {
  q?: string;
  subject?: string;
  sort?: SearchSort;
  page?: number;
  limit?: number;
};
