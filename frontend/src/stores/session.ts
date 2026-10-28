import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";
import type { RegisterIn } from "../api/generated";
import { resetDiscoverBrowseCache } from "../pages/discoverCache";
import type { User } from "../types";
import { useClubs } from "./clubs";
import { useLocale } from "./locale";
import { browserTimeZone } from "../timezone";
import { useTheme } from "./theme";

const INVALID_TIME_ZONE = "That is not a valid time zone";

/** The server refused the zone (one message of a 400 joined by "; "). */
function refusedZone(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    error.status === 400 &&
    typeof error.detail === "string" &&
    error.detail.split("; ").includes(INVALID_TIME_ZONE)
  );
}

/**
 * Whatever belonged to the previous account: the clubs and Discover's
 * browse snapshot, whose hits carry that viewer's shelf and club badges.
 */
function forgetAccountState() {
  useClubs().clear();
  resetDiscoverBrowseCache();
}

export const useSession = defineStore("session", {
  state: () => ({
    user: null as User | null,
    ready: false,
  }),
  actions: {
    async hydrate() {
      if (this.ready) return;
      try {
        this.user = await api.me();
        useTheme().adopt(this.user);
        useLocale().adopt(this.user);
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          throw error;
        }
        this.user = null;
      } finally {
        this.ready = true;
      }
    },
    async register(body: { username: string; email: string; password: string; inviteCode: string }) {
      const locale = useLocale();
      const request: RegisterIn = {
        username: body.username,
        email: body.email,
        password: body.password,
        invite_code: body.inviteCode || null,
        timezone: browserTimeZone(),
        locale: locale.locale,
      };
      try {
        this.user = await api.register(request);
      } catch (error) {
        // The form has no zone field, so a zone the server does not know
        // (`Etc/Unknown`, a name newer or older than its table) must not
        // block sign-up: the account starts in UTC, and the reader picks
        // their zone in Settings.
        if (!request.timezone || !refusedZone(error)) throw error;
        this.user = await api.register({ ...request, timezone: null });
      }
      forgetAccountState();
      // A new account has no stored preference yet, so push the one this
      // browser has been using rather than snapping back to the default.
      const theme = useTheme();
      void theme.persist({ theme: theme.theme, color_mode: theme.mode });
      void useLocale().persist();
    },
    async login(username: string, password: string) {
      await api.login({ username, password });
      forgetAccountState();
      this.user = await api.me();
      useTheme().adopt(this.user);
      const locale = useLocale();
      if (locale.explicit) {
        // Auth-screen EN|DE was an explicit choice; keep it (existing
        // members are backfilled to `en`, so adopt() would undo the toggle).
        void locale.persist();
      } else {
        locale.adopt(this.user);
      }
    },
    async logout() {
      try {
        await api.logout();
      } finally {
        this.reset();
      }
    },
    /** Forget the signed-in user, after a logout or when the server ended the session. */
    reset() {
      this.user = null;
      forgetAccountState();
      useLocale().clearExplicit();
    },
  },
});
