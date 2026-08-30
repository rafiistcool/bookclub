import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";
import type { User } from "../types";
import { useTheme } from "./theme";

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
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          throw error;
        }
        this.user = null;
      } finally {
        this.ready = true;
      }
    },
    async register(username: string, password: string, inviteCode: string) {
      this.user = await api.register({
        username,
        password,
        invite_code: inviteCode,
      });
      // A new account has no stored preference yet, so push the one this
      // browser has been using rather than snapping back to the default.
      const theme = useTheme();
      void theme.persist({ theme: theme.theme, color_mode: theme.mode });
    },
    async login(username: string, password: string) {
      await api.login({ username, password });
      this.user = await api.me();
      useTheme().adopt(this.user);
    },
    async logout() {
      try {
        await api.logout();
      } finally {
        this.user = null;
      }
    },
  },
});
