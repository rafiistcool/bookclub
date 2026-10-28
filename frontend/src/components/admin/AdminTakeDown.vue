<script setup lang="ts">
import { vAutosize } from "../../autosize";
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import { fromLocalInput, pictureRef, TAKEDOWN_DETAILS, toLocalInput } from "../../admin";
import { useToast } from "../../stores/toast";
import BookCover from "../BookCover.vue";

/**
 * "Take down a picture by URL": how a notice that arrived by email (from
 * someone without an account here) enters the record. The admin pastes the
 * picture's address, a book page (which means its cover) or a picture id,
 * checks the picture, and confirms. The server takes the picture down and
 * records the notice as a resolved report that arrived when the notice did.
 * A notice about a picture that is not a cover (an avatar) cannot be recorded
 * yet: the server refuses it and takes nothing down, and the form then offers
 * the takedown without a record.
 */
const emit = defineEmits<{ "taken-down": [] }>();

const REASONS = ["copyright", "offensive", "wrong_book", "other"] as const;
/** How far ahead of this device's clock an arrival time may be (the server allows 5 minutes). */
const SLACK_MS = 5 * 60_000;

const { t } = useI18n();
const toast = useToast();

const url = ref("");
const receivedAt = ref(toLocalInput(new Date()));
const reason = ref<(typeof REASONS)[number]>("copyright");
const details = ref("");
/** The picture the form names, once checked: shown before the second tap. */
const target = ref<{ publicId: string; title: string; thumb: string | null; full: string | null } | null>(null);
const problem = ref("");
const busy = ref(false);
/** The server refused to record a notice for this picture; the next tap takes it down without one. */
const withoutRecord = ref(false);

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

function reset() {
  url.value = "";
  receivedAt.value = toLocalInput(new Date());
  reason.value = "copyright";
  details.value = "";
  target.value = null;
  problem.value = "";
  withoutRecord.value = false;
}

/** Any change to the address asks for a new check. */
function edited() {
  target.value = null;
  problem.value = "";
  withoutRecord.value = false;
}

function cancel() {
  target.value = null;
  withoutRecord.value = false;
}

/** The server's 400s for a notice, in this form's words. */
function refused(err: unknown): string {
  const detail = err instanceof ApiError && err.status === 400 ? err.detail : null;
  if (detail === TAKEDOWN_DETAILS.receivedInFuture) return t("admin.takeDownForm.future");
  if (detail === TAKEDOWN_DETAILS.coversOnly) {
    withoutRecord.value = true;
    return t("admin.takeDownForm.coversOnly");
  }
  return failed(err, t("admin.takeDownForm.failed"));
}

/** The first tap: find the picture the address names and show it. */
async function check() {
  problem.value = "";
  const found = pictureRef(url.value);
  if (!found) {
    problem.value = t("admin.takeDownForm.notAPicture");
    return;
  }
  if (found.kind === "media") {
    const id = found.publicId;
    target.value = { publicId: id, title: "", thumb: `/media/${id}/s`, full: `/media/${id}/l` };
    return;
  }
  busy.value = true;
  try {
    const book = await api.book(found.book);
    if (!book.cover_public_id) {
      problem.value = t("admin.takeDownForm.noCover");
      return;
    }
    target.value = {
      publicId: book.cover_public_id,
      title: book.title,
      thumb: book.cover_thumb_url,
      full: book.cover_url,
    };
  } catch (err) {
    problem.value = failed(err, t("admin.takeDownForm.notAPicture"));
  } finally {
    busy.value = false;
  }
}

/** The second tap: take it down and record the notice. */
async function takeDown() {
  if (!target.value || busy.value) return;
  const received = fromLocalInput(receivedAt.value);
  if (received && new Date(received).getTime() > Date.now() + SLACK_MS) {
    problem.value = t("admin.takeDownForm.future");
    return;
  }
  busy.value = true;
  try {
    await api.adminTakeDown(target.value.publicId, {
      reason: reason.value,
      details: details.value.trim(),
      ...(received ? { received_at: received } : {}),
    });
    toast.show(t("admin.takeDownForm.done"));
    reset();
    emit("taken-down");
  } catch (err) {
    problem.value = refused(err);
  } finally {
    busy.value = false;
  }
}

/** After the server refused the notice: the takedown alone, which records nothing. */
async function takeDownWithoutRecord() {
  if (!target.value || busy.value) return;
  busy.value = true;
  try {
    await api.adminTakeDown(target.value.publicId);
    toast.show(t("admin.takeDownForm.doneWithoutRecord"));
    reset();
    emit("taken-down");
  } catch (err) {
    problem.value = failed(err, t("admin.takeDownForm.failed"));
  } finally {
    busy.value = false;
  }
}

function submit() {
  if (target.value && withoutRecord.value) void takeDownWithoutRecord();
  else if (target.value) void takeDown();
  else void check();
}
</script>

<template>
  <details class="takedown panel">
    <summary>{{ t("admin.takeDownForm.toggle") }}</summary>
    <form class="stack takedown-form" @submit.prevent="submit">
      <p class="fine muted">{{ t("admin.takeDownForm.blurb") }}</p>
      <label class="field">
        <span>{{ t("admin.takeDownForm.url") }}</span>
        <input
          v-model="url"
          name="takedown-url"
          type="text"
          inputmode="url"
          autocomplete="off"
          spellcheck="false"
          required
          :placeholder="t('admin.takeDownForm.urlPlaceholder')"
          @input="edited"
        />
      </label>
      <div class="takedown-row">
        <label class="field">
          <span>{{ t("admin.takeDownForm.receivedAt") }}</span>
          <input v-model="receivedAt" name="received-at" type="datetime-local" :max="toLocalInput(new Date())" />
        </label>
        <label class="field">
          <span>{{ t("admin.takeDownForm.reason") }}</span>
          <select v-model="reason" name="reason">
            <option v-for="code in REASONS" :key="code" :value="code">{{ t(`cover.reasons.${code}`) }}</option>
          </select>
        </label>
      </div>
      <label class="field">
        <span>{{ t("admin.takeDownForm.details") }}</span>
        <textarea v-model="details" v-autosize name="details" rows="2" maxlength="2000" :placeholder="t('common.optional')" />
      </label>

      <div v-if="target" class="takedown-target">
        <BookCover :book="{ title: target.title, cover_url: target.full, cover_thumb_url: target.thumb }" size="sm" />
        <p class="fine">
          <strong v-if="target.title">{{ target.title }}</strong>
          {{ t("admin.takeDownBlurb") }}
        </p>
      </div>
      <p v-if="problem" class="error fine" role="alert">{{ problem }}</p>
      <div class="btn-row">
        <button v-if="target" class="btn btn-danger btn-sm" type="submit" :disabled="busy">
          {{ withoutRecord ? t("admin.takeDownForm.withoutRecord") : t("admin.takeDownConfirm") }}
        </button>
        <button v-else class="btn btn-ghost btn-sm" type="submit" :disabled="busy || !url.trim()">
          {{ t("admin.takeDownForm.check") }}
        </button>
        <button v-if="target" class="btn btn-ghost btn-sm" type="button" @click="cancel">
          {{ t("common.cancel") }}
        </button>
      </div>
    </form>
  </details>
</template>

<style scoped>
.takedown {
  margin: 0 0 var(--space-3);
}

.takedown summary {
  cursor: pointer;
  font-weight: 600;
}

.takedown-form {
  margin-top: var(--space-3);
}

.takedown-form p {
  margin: 0;
}

.takedown-row {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
  gap: var(--space-3);
}

.takedown-row input,
.takedown-row select {
  min-width: 0;
  width: 100%;
}

.takedown-target {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}

.takedown-target strong {
  display: block;
}
</style>
