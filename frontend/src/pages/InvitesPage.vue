<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import { useToast } from "../stores/toast";
import type { Invite } from "../types";

const invites = ref<Invite[]>([]);
const error = ref("");
const pending = ref(false);
const toast = useToast();

async function load() {
  try {
    invites.value = await api.invites();
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load invites";
  }
}

async function mint() {
  pending.value = true;
  try {
    const created = await api.createInvite();
    await load();
    await copy(created.code);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not create an invite");
  } finally {
    pending.value = false;
  }
}

async function copy(code: string) {
  try {
    await navigator.clipboard.writeText(code);
    toast.show("Invite copied");
  } catch {
    toast.show(code);
  }
}

onMounted(load);
</script>

<template>
  <section>
    <h1>Invites</h1>
    <p class="lede">Anyone with a code can make an account. Treat them like passwords.</p>
    <button class="btn btn-primary" type="button" :disabled="pending" @click="mint">
      {{ pending ? "Creating…" : "Create invite" }}
    </button>
    <p v-if="error" class="error" style="margin-top: 16px">{{ error }}</p>
    <div style="margin-top: 20px">
      <div v-for="invite in invites" :key="invite.code" class="invite-row">
        <div>
          <div class="invite-code">{{ invite.code }}</div>
          <p class="muted fine">
            <template v-if="invite.used">Used by {{ invite.used_by }}</template>
            <template v-else>Unused</template>
          </p>
        </div>
        <button v-if="!invite.used" class="btn btn-ghost" type="button" @click="copy(invite.code)">
          Copy
        </button>
      </div>
      <p v-if="invites.length === 0" class="muted">No invites yet.</p>
    </div>

    <section id="backup" class="backup-block">
      <h2>Export / backup</h2>
      <p class="muted fine" style="margin-bottom: 12px">
        Download a copy of the club SQLite database. Keep it somewhere safe — this does not restore
        from a file.
      </p>
      <a class="btn btn-ghost" href="/api/backup">Download database</a>
    </section>
  </section>
</template>
