<script setup lang="ts">
import { ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "../api/client";
import { useSession } from "../stores/session";

const session = useSession();
const router = useRouter();
const route = useRoute();
const username = ref("");
const password = ref("");
const show = ref(false);
const error = ref("");
const pending = ref(false);

function safeNextPath(raw: unknown): string {
  if (typeof raw !== "string") return "/library";
  if (!raw.startsWith("/") || raw.startsWith("//") || raw.includes("\\")) {
    return "/library";
  }
  return raw;
}

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.login(username.value, password.value);
    await router.replace(safeNextPath(route.query.next));
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not sign in";
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">Bookclub</h1>
    <p class="lede">Sign in to your shelf.</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>Username</span>
        <input v-model="username" name="username" autocomplete="username" required />
      </label>
      <label class="field">
        <span>Password</span>
        <div class="password-wrap">
          <input
            v-model="password"
            :type="show ? 'text' : 'password'"
            name="password"
            autocomplete="current-password"
            required
          />
          <button class="text-btn" type="button" @click="show = !show">
            {{ show ? "Hide" : "Show" }}
          </button>
        </div>
      </label>
      <button class="btn btn-primary" type="submit" :disabled="pending">
        {{ pending ? "Signing in…" : "Sign in" }}
      </button>
    </form>
    <p class="muted fine" style="margin-top: 18px">
      Have an invite?
      <RouterLink to="/register">Create an account</RouterLink>
    </p>
  </main>
</template>
