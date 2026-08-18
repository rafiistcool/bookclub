import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";
import type { User } from "../types";

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
    },
    async login(username: string, password: string) {
      await api.login({ username, password });
      this.user = await api.me();
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
