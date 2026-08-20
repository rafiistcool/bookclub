<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { ApiError } from "../api/client";
import { useClub } from "../stores/club";
import { useSession } from "../stores/session";

const club = useClub();

const session = useSession();
const router = useRouter();
const username = ref("");
const password = ref("");
const invite = ref("");
const show = ref(false);
const error = ref("");
const pending = ref(false);

async function submit() {
  error.value = "";
  pending.value = true;
  try {
    await session.register(username.value, password.value, invite.value);
    await router.replace("/library");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not create account";
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <h1 class="wordmark">{{ club.name }}</h1>
    <p class="lede">Join with an invite from someone already here.</p>
    <form @submit.prevent="submit">
      <p v-if="error" class="error">{{ error }}</p>
      <label class="field">
        <span>Invite code</span>
        <input
          v-model="invite"
          name="invite"
          autocomplete="one-time-code"
          autocapitalize="characters"
          required
        />
      </label>
      <label class="field">
        <span>Username</span>
        <input
          v-model="username"
          name="username"
          autocomplete="username"
          spellcheck="false"
          required
        />
      </label>
      <label class="field">
        <span>Password</span>
        <div class="password-wrap">
          <input
            v-model="password"
            :type="show ? 'text' : 'password'"
            name="password"
            autocomplete="new-password"
            minlength="8"
            required
          />
          <button class="text-btn" type="button" @click="show = !show">
            {{ show ? "Hide" : "Show" }}
          </button>
        </div>
      </label>
      <button class="btn btn-primary" type="submit" :disabled="pending">
        {{ pending ? "Creating…" : "Create account" }}
      </button>
    </form>
    <p class="muted fine" style="margin-top: 18px">
      Already have an account?
      <RouterLink to="/login">Sign in</RouterLink>
    </p>
  </main>
</template>
