import { t } from "./index";

const API_ERROR_KEYS: Record<string, string> = {
  "Not signed in": "errors.notSignedIn",
  "That username is already taken": "errors.usernameTaken",
  "That invite code is not valid": "errors.inviteInvalid",
  "Too many attempts. Wait a few minutes.": "errors.tooManyAttempts",
  "Wrong username or password": "errors.wrongCredentials",
  "Current password is wrong": "errors.currentPasswordWrong",
  "Pick a different password": "errors.differentPassword",
  "That picture is too large (2 MB max)": "errors.pictureTooLarge",
  "Use a JPEG, PNG, WebP, or GIF": "errors.usePicture",
  "That file is not a picture": "errors.notAPicture",
  "No profile picture": "errors.noPicture",
};

export function translateApiError(detail: string): string {
  const key = API_ERROR_KEYS[detail];
  return key ? t(key) : detail;
}
