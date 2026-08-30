<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import NextUpVote from "../components/NextUpVote.vue";
import { bookPath, STATUS_SHORT } from "../constants";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { Member, OverlapBook } from "../types";

const session = useSession();
const toast = useToast();

const members = ref<Member[]>([]);
const membersError = ref("");
const membersLoaded = ref(false);

const overlap = ref<OverlapBook[]>([]);
const overlapError = ref("");
const overlapLoaded = ref(false);
const includeReading = ref(false);
const nominating = ref<string | null>(null);

const voteSection = ref<InstanceType<typeof NextUpVote> | null>(null);

async function loadMembers() {
  try {
    members.value = await api.members();
    membersError.value = "";
  } catch (err) {
    membersError.value =
      err instanceof ApiError ? err.message : "Could not load the club";
  } finally {
    membersLoaded.value = true;
  }
}

async function loadOverlap() {
  overlapLoaded.value = false;
  try {
    overlap.value = (await api.overlap(includeReading.value)).items;
    overlapError.value = "";
  } catch (err) {
    overlap.value = [];
    overlapError.value =
      err instanceof ApiError ? err.message : "Could not load the overlap";
  } finally {
    overlapLoaded.value = true;
  }
}

async function nominate(row: OverlapBook) {
  nominating.value = row.book.ol_work_key;
  try {
    await api.nominate({
      ol_work_key: row.book.ol_work_key,
      title: row.book.title,
      authors: row.book.authors,
      cover_id: row.book.cover_id,
      year: row.book.year,
    });
    toast.show("Nominated for the next-up vote");
    await voteSection.value?.load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
  } finally {
    nominating.value = null;
  }
}

onMounted(() => {
  void loadMembers();
  void loadOverlap();
});

watch(includeReading, loadOverlap);
</script>

<template>
  <section>
    <div class="page-head">
      <h1>Club</h1>
      <p class="lede">Who's here, what you have in common, and what to read next.</p>
    </div>

    <section aria-labelledby="members">
      <div class="section-head">
        <h2 id="members">Members</h2>
        <span v-if="membersLoaded" class="fine subtle nums">
          {{ members.length + 1 }}
        </span>
      </div>

      <p v-if="membersError" class="error">{{ membersError }}</p>
      <div v-else-if="!membersLoaded" class="skeleton member-skeleton" aria-hidden="true" />

      <div v-else-if="members.length === 0" class="empty">
        <h3>You're the only one here</h3>
        <p>Mint an invite code and send it to someone. A club of one is a book.</p>
        <div class="btn-row">
          <RouterLink class="btn btn-primary" to="/settings">Create an invite</RouterLink>
        </div>
      </div>

      <ul v-else class="member-list">
        <li v-if="session.user" class="member-card is-you">
          <span class="member-meta">
            <strong>{{ session.user.username }}</strong>
            <span class="finer subtle">That's you</span>
          </span>
          <RouterLink class="btn btn-ghost btn-sm" to="/shelf">Your shelf</RouterLink>
        </li>
        <li v-for="member in members" :key="member.username">
          <RouterLink class="member-card" :to="`/club/${member.username}`">
            <span class="member-meta">
              <strong>{{ member.username }}</strong>
              <span class="finer subtle nums">
                {{ member.currently_reading_count }} currently reading
              </span>
            </span>
            <span class="preview-row">
              <BookCover
                v-for="book in member.currently_reading_preview"
                :key="book.title"
                :title="book.title"
                :cover-id="book.cover_id"
                size="xs"
              />
            </span>
          </RouterLink>
        </li>
      </ul>
    </section>

    <section class="section" aria-labelledby="overlap">
      <div class="section-head">
        <h2 id="overlap">TBR overlap</h2>
        <button
          class="chip"
          type="button"
          :aria-pressed="includeReading"
          :class="{ active: includeReading }"
          @click="includeReading = !includeReading"
        >
          Include Reading
        </button>
      </div>
      <p class="fine muted overlap-blurb">
        Books two or more of you have on Want to read{{
          includeReading ? " or Reading" : ""
        }}. The shortest path to a pick everyone already wants.
      </p>

      <p v-if="overlapError" class="error">{{ overlapError }}</p>
      <div v-else-if="!overlapLoaded" class="skeleton member-skeleton" aria-hidden="true" />

      <p v-else-if="overlap.length === 0" class="fine subtle">
        No shared wants yet. Add a few books to Want to read and check back.
      </p>

      <ul v-else class="overlap-list">
        <li v-for="row in overlap" :key="row.book.ol_work_key" class="overlap-row">
          <RouterLink class="overlap-book" :to="bookPath(row.book.ol_work_key)">
            <BookCover :title="row.book.title" :cover-id="row.book.cover_id" size="sm" />
            <span class="overlap-meta">
              <strong>{{ row.book.title }}</strong>
              <span v-if="row.book.authors" class="finer subtle">
                {{ row.book.authors }}
              </span>
              <span class="meta-line">
                <span
                  v-for="member in row.members"
                  :key="member.username"
                  class="badge"
                  :class="member.status"
                >
                  {{ member.username }} · {{ STATUS_SHORT[member.status] }}
                </span>
              </span>
            </span>
          </RouterLink>
          <button
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="nominating === row.book.ol_work_key"
            @click="nominate(row)"
          >
            Nominate
          </button>
        </li>
      </ul>
    </section>

    <section class="section">
      <NextUpVote ref="voteSection" />
    </section>
  </section>
</template>

<style scoped>
.member-skeleton {
  height: 130px;
}

.member-list,
.overlap-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.member-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  text-decoration: none;
  color: inherit;
}

.member-card:hover {
  border-color: var(--border-strong);
}

.is-you {
  border-style: dashed;
}

.member-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.member-meta strong {
  font-family: var(--serif);
  font-size: var(--text-lg);
}

.preview-row {
  display: flex;
  gap: var(--space-1);
  flex: 0 0 auto;
}

.overlap-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.overlap-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.overlap-book {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: var(--space-3);
  color: inherit;
  text-decoration: none;
}

.overlap-meta {
  display: grid;
  gap: 2px;
  align-content: start;
  min-width: 0;
}

.overlap-meta strong {
  font-family: var(--serif);
  font-size: var(--text-md);
  line-height: var(--leading-snug);
}

@media (min-width: 1024px) {
  .member-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
