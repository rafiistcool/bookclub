import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";
import type { RegisterIn } from "../api/generated";
import { forgetDrafts } from "../drafts";
import { resetDiscoverBrowseCache } from "../pages/discoverCache";
import type { User } from "../types";
import { forgetLastClub, useClubs } from "./clubs";
import { useLocale } from "./locale";
import { browserTimeZone } from "../timezone";
import { useTheme } from "./theme";

const INVALID_TIME_ZONE = "That is not a valid time zone";

/**
 * The server's 400 when a sign-up or `POST /api/auth/terms` sent a terms
 * version that is no longer the current one (`msg::TERMS_CHANGED`).
 */
export const TERMS_CHANGED = "The terms of use have changed. Reload the page and try again.";

/** True for the 400 that says the terms changed since the page loaded them. */
export function isTermsChanged(error: unknown): boolean {
  return (
    error !== null &&
    typeof error === "object" &&
    (error as { status?: unknown }).status === 400 &&
    (error as { detail?: unknown }).detail === TERMS_CHANGED
  );
}

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
    /**
     * Creates the account and signs it in. `termsVersion` is the version of
     * the terms the sign-up page showed (`/api/config.terms_version`); the
     * server stores it as the acceptance, and refuses a missing or old one
     * while terms are configured.
     */
    async register(body: {
      username: string;
      email: string;
      password: string;
      inviteCode: string;
      termsVersion?: string | null;
    }) {
      const locale = useLocale();
      const request: RegisterIn = {
        username: body.username,
        email: body.email,
        password: body.password,
        invite_code: body.inviteCode || null,
        timezone: browserTimeZone(),
        locale: locale.locale,
        terms_version: body.termsVersion ?? null,
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
    /** Accepts `termsVersion`, the current terms, for the signed-in account (`UserOut.needs_terms`). */
    async acceptTerms(termsVersion: string) {
      this.user = await api.acceptTerms(termsVersion);
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
        this.reset({ leavingDevice: true });
      }
    },
    /**
     * Forget the signed-in user, after a logout or when the server ended the
     * session. `leavingDevice` (the reader signed out here, ended this
     * session or deleted the account) also forgets what the account left on
     * this device: unsent notes and the last club. An expired session keeps
     * them for the reader's next sign-in.
     */
    reset(options: { leavingDevice?: boolean } = {}) {
      this.user = null;
      forgetAccountState();
      useLocale().clearExplicit();
      if (options.leavingDevice) {
        forgetDrafts();
        forgetLastClub();
        useClubs().clear();
      }
    },
  },
});
