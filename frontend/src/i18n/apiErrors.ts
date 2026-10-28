import { t } from "./index";

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

  // Pictures
  "That picture is too large (50 MB max)": "errors.pictureTooLarge",
  "That picture has too many pixels": "errors.pictureTooManyPixels",
  "Use a JPEG, PNG, WebP, or GIF": "errors.usePicture",
  "That file is not a picture": "errors.notAPicture",
  "This picture was removed": "errors.pictureRemoved",

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

  // Catalog
  "No such book.": "errors.noSuchBook",
  "That book is not in the catalog": "errors.bookNotInCatalog",
  "That edition belongs to another book": "errors.editionOfAnotherBook",
  "That ISBN is already in the catalog": "errors.isbnInCatalog",
  "That is not a valid ISBN-10 or ISBN-13": "errors.invalidIsbn",
  "Could not look that ISBN up right now. Try again.": "errors.isbnLookupFailed",
  "This book was added by a reader, not Open Library.": "errors.addedByReader",
  "A book cannot be merged into itself": "errors.mergeIntoItself",
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
};

/**
 * Details the server builds from a template. The first capture group, if any,
 * reaches the translation as `{n}`.
 */
const API_ERROR_PATTERNS: ReadonlyArray<readonly [RegExp, string]> = [
  [/^The library is busy\. Try again in (\d+) seconds\.$/, "errors.libraryBusyIn"],
];

export function translateApiError(detail: string): string {
  const key = API_ERROR_KEYS[detail];
  if (key) return t(key);
  for (const [pattern, patternKey] of API_ERROR_PATTERNS) {
    const match = pattern.exec(detail);
    if (match) return t(patternKey, { n: match[1] ?? "" });
  }
  return detail;
}
