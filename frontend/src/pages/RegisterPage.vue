<script setup lang="ts">
import { ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ApiError } from "../api/client";
import { useClub } from "../stores/club";
import { useSession } from "../stores/session";

const club = useClub();
const session = useSession();
const router = useRouter();
const route = useRoute();
const username = ref("");
const password = ref("");
const invite = ref(typeof route.query.invite === "string" ? route.query.invite : "");
const show = ref(false);
const error = ref("");
const pending = ref(false);

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.register(username.value.trim(), password.value, invite.value.trim());
    await router.replace("/");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not create account";
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <p class="kicker">Join the club</p>
    <h1 class="wordmark" style="font-size: var(--text-2xl)">{{ club.name }}</h1>
    <p class="lede">You need an invite code from someone already here.</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error fine" role="alert" style="margin-bottom: 10px">{{ error }}</p>
      <label class="field">
        <span>Invite code</span>
        <input v-model="invite" name="invite" autocomplete="one-time-code" autocapitalize="characters" spellcheck="false" required />
      </label>
      <label class="field">
        <span>Username</span>
        <input v-model="username" name="username" autocomplete="username" autocapitalize="none" spellcheck="false" required />
        <span class="field-hint">2–32 characters: a–z, 0–9, underscore.</span>
      </label>
      <label class="field">
        <span>Password</span>
        <div class="password-wrap">
          <input v-model="password" :type="show ? 'text' : 'password'" name="password" autocomplete="new-password" minlength="8" required />
          <button class="text-btn sm" type="button" @click="show = !show">{{ show ? "Hide" : "Show" }}</button>
        </div>
        <span class="field-hint">At least 8 characters. There is no reset by email — keep it somewhere safe.</span>
      </label>
      <button class="btn btn-primary btn-block" type="submit" :disabled="pending">
        {{ pending ? "Creating…" : "Create account" }}
      </button>
    </form>
    <p class="muted fine" style="margin-top: 18px">
      Already have an account? <RouterLink to="/login">Sign in</RouterLink>
    </p>
  </main>
</template>
