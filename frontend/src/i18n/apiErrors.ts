import { t, te } from "./index";

/**
 * The server answers errors with an English `detail`. Each key here is a
 * byte-for-byte copy of one the server sends (server/tests/api_error_copy.rs
 * checks that every key still exists in server/src), mapped to a translation.
 * Strings that are not listed show as the server wrote them.
 */
const API_ERROR_KEYS: Record<string, string> = {
  // Accounts and sign-in
  "Not signed in": "errors.notSignedIn",
  "That username is already taken": "errors.usernameTaken",
  "Username must be 2–32 characters: lowercase letters, numbers, underscore": "errors.usernameRule",
  "Password must be at least 8 characters": "errors.passwordShort",
  "That invite code is not valid": "errors.inviteInvalid",
  "Sign-up needs an invite code right now": "errors.signupNeedsInvite",
  "Too many attempts. Wait a few minutes.": "errors.tooManyAttempts",
  "Wrong username or password": "errors.wrongCredentials",
  "Current password is wrong": "errors.currentPasswordWrong",
  "Pick a different password": "errors.differentPassword",
  "New password must be at least 8 characters": "errors.newPasswordShort",
  "That email is already in use": "errors.emailInUse",
  "Enter a valid email address": "errors.invalidEmail",
  "That link has expired or was already used": "errors.linkExpired",
  "Add an email address first": "errors.addEmailFirst",
  "Verify your email address first": "errors.verifyEmailFirst",
  "That is not a valid time zone": "errors.invalidTimeZone",
  "Transfer or delete your clubs first": "errors.transferClubsFirst",
  "No such session": "errors.noSuchSession",
  "The terms of use have changed. Reload the page and try again.": "errors.termsChanged",

  // Pictures
  "That picture is too large (50 MB max)": "errors.pictureTooLarge",
  "That picture has too many pixels": "errors.pictureTooManyPixels",
  "Use a JPEG, PNG, WebP, or GIF": "errors.usePicture",
  "That file is not a picture": "errors.notAPicture",
  "This picture was removed": "errors.pictureRemoved",
  "This report was already resolved": "errors.reportResolved",

  // Clubs
  "No such club": "errors.noSuchClub",
  "Only club admins can do that": "errors.adminsOnly",
  "Only the club owner can do that": "errors.ownerOnly",
  "This club is full": "errors.clubFull",
  "You are already in this club": "errors.alreadyMember",
  "You can create at most 10 clubs": "errors.clubLimit",
  "Transfer ownership before leaving": "errors.transferBeforeLeaving",
  "Use transfer to change the owner": "errors.useTransfer",
  "Use leave to leave the club": "errors.useLeave",
  "Give the club a name": "errors.clubNameRequired",
  "Keep the club name to 80 characters": "errors.clubNameTooLong",
  "No such invite": "errors.noSuchInvite",
  "No member with that name": "errors.noMember",
  "Percent milestones go up to 100": "errors.percentMax",
  "A range needs a unit": "errors.rangeNeedsUnit",
  "Replies and club posts cannot be private": "errors.repliesNotPrivate",
  "No entry with that id": "errors.noEntry",
  "That entry is not on this book": "errors.entryNotOnBook",
  "You can only edit your own entries": "errors.editOwnEntries",
  "You can only delete your own entries": "errors.deleteOwnEntries",

  // Club picks, milestones and the next-up vote
  "No club pick yet": "errors.noPickYet",
  "No club pick with that id": "errors.noPickWithId",
  "That pick is closed": "errors.pickClosed",
  "No meeting scheduled": "errors.noMeeting",
  "note must be 280 characters or fewer": "errors.noteTooLong",
  "meeting_at must be a date and time": "errors.meetingFormat",
  "No milestone with that id": "errors.noMilestone",
  "Give the milestone a title": "errors.milestoneTitle",
  "Keep the title to 120 characters": "errors.milestoneTitleLong",
  "Chapter range is backwards": "errors.rangeBackwards",
  "That milestone is not on this pick": "errors.milestoneNotOnPick",
  "Nominate a book first": "errors.nominateFirst",
  "The deadline has to be in the future": "errors.deadlineFuture",
  "This vote already has enough books": "errors.voteFull",
  "That book is already the club pick": "errors.alreadyPick",
  "That book is already nominated": "errors.alreadyNominated",
  "No open next-up vote": "errors.noOpenVote",
  "No nomination with that id": "errors.noNomination",

  // Catalog
  "No such book.": "errors.noSuchBook",
  "That book is not in the catalog": "errors.bookNotInCatalog",
  "That edition belongs to another book": "errors.editionOfAnotherBook",
  "That ISBN is already in the catalog": "errors.isbnInCatalog",
  "A book cannot be merged into itself": "errors.mergeIntoItself",
  "This request was already decided": "errors.requestDecided",
  "That is not a valid ISBN-10 or ISBN-13": "errors.invalidIsbn",
  "Could not look that ISBN up right now. Try again.": "errors.isbnLookupFailed",
  "This book was added by a reader, not Open Library.": "errors.addedByReader",
  "A title is required": "errors.titleRequired",
  "Need at least 3 characters. Add the author, or paste an ISBN.": "errors.searchTooShort",
  "Search is too long": "errors.searchTooLong",
  "Could not search the library right now. Try again.": "errors.searchUnavailable",
  "Could not load the library right now. Try again.": "errors.libraryUnavailable",
  "The library is busy. Wait a few seconds and try again.": "errors.libraryBusy",

  // Shelf, favourites, search history and the Goodreads import
  "Already on your shelf": "errors.alreadyOnShelf",
  "Book not on your shelf": "errors.notOnShelf",
  "Reading date requires a finished book": "errors.readingDateFinished",
  "Reading date cannot be in the future": "errors.readingDateFuture",
  "Each book can only be a favourite once": "errors.duplicateFavourite",
  "At most 3 favourites": "errors.tooManyFavourites",
  "You already have 3 favourites": "errors.favouritesFull",
  "Search history entry not found": "errors.historyNotFound",
  "Paste the line you want to keep": "errors.quoteEmpty",
  "Keep it to 1000 characters": "errors.quoteTooLong",
  "No quote with that id": "errors.noQuote",
  "You can only edit your own quotes": "errors.editOwnQuotes",
  "You can only delete your own quotes": "errors.deleteOwnQuotes",
  "An import is already running": "errors.importRunning",
  "No such import": "errors.noSuchImport",
  "The import stopped unexpectedly. Try again.": "errors.importStopped",
  "That file is too large. Keep the export under 1.5 MB.": "errors.importTooLarge",
  "The file must be a UTF-8 Goodreads CSV export": "errors.importNotUtf8",
  "That does not look like a Goodreads export": "errors.importNotGoodreads",

  // Platform
  "Slow down a little. Try again later.": "errors.slowDown",
  "Cross-site request refused": "errors.crossSite",
  "Request body is too large": "errors.bodyTooLarge",
  "The request took too long": "errors.requestTimeout",
  "The server is busy. Try again in a moment.": "errors.serverBusy",
};

/** The edit-request field a template names, by the form's label; an unknown key as it is. */
function fieldLabel(key: string): string {
  const label = `editRequest.fields.${key}`;
  return te(label) ? t(label) : key;
}

/**
 * Details the server builds from a template, with what the translation gets
 * from the match.
 */
const API_ERROR_PATTERNS: ReadonlyArray<
  readonly [RegExp, string, (match: RegExpExecArray) => Record<string, string>]
> = [
  [/^The library is busy\. Try again in (\d+) seconds\.$/, "errors.libraryBusyIn", (m) => ({ n: m[1]! })],
  // An edit request's value that `apply_admin_fields` refuses.
  [/^Invalid value for ([a-z_]+)$/, "errors.invalidValueFor", (m) => ({ field: fieldLabel(m[1]!) })],
];

export function translateApiError(detail: string): string {
  const key = API_ERROR_KEYS[detail];
  if (key) return t(key);
  for (const [pattern, patternKey, params] of API_ERROR_PATTERNS) {
    const match = pattern.exec(detail);
    if (match) return t(patternKey, params(match));
  }
  return detail;
}
