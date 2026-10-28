<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../../api/client";
import BookCover from "../../components/BookCover.vue";
import NextUpVote from "../../components/NextUpVote.vue";
import { bookPath } from "../../constants";
import { isClubAdmin, isClubGone, useClubs, useCurrentClub } from "../../stores/clubs";
import { useToast } from "../../stores/toast";
import { suggestionReason } from "../../suggestions";
import type { VoteSuggestionOut } from "../../types";

const { t } = useI18n();
const clubs = useClubs();
const toast = useToast();
const router = useRouter();

const club = useCurrentClub();
const slug = computed(() => club.value.slug);
const canManage = computed(() => isClubAdmin(club.value.role));

const voteSection = ref<InstanceType<typeof NextUpVote> | null>(null);
const suggestions = ref<VoteSuggestionOut[]>([]);
const suggestionsLoaded = ref(false);
const nominating = ref<number | null>(null);

async function loadSuggestions() {
  try {
    suggestions.value = (await api.voteSuggestions(slug.value)).items;
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(slug.value);
    suggestions.value = [];
  } finally {
    suggestionsLoaded.value = true;
  }
}

async function nominate(row: VoteSuggestionOut) {
  nominating.value = row.book.id;
  try {
    await api.nominate(slug.value, row.book.id);
    toast.show(t("shelf.nominated"));
    suggestions.value = suggestions.value.filter((item) => item.book.id !== row.book.id);
    await voteSection.value?.load();
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(slug.value);
    toast.show(err instanceof ApiError ? err.message : t("shelf.nominateFailed"));
  } finally {
    nominating.value = null;
  }
}

/** The winner is the club pick now: show it. */
function applied() {
  void router.push(`/c/${slug.value}`);
}

onMounted(loadSuggestions);
</script>

<template>
  <div class="vote-page">
    <NextUpVote ref="voteSection" :slug="slug" :can-manage="canManage" @applied="applied" />

    <section v-if="suggestionsLoaded && suggestions.length" class="section" aria-labelledby="suggestions">
      <div class="section-head">
        <h2 id="suggestions">{{ t("vote.suggestions") }}</h2>
      </div>
      <p class="fine muted suggestions-blurb">{{ t("vote.suggestionsBlurb") }}</p>
      <ul class="suggestion-list">
        <li v-for="row in suggestions" :key="row.book.id" class="suggestion">
          <RouterLink class="suggestion-book" :to="bookPath(row.book)">
            <BookCover :book="row.book" size="sm" />
            <span class="suggestion-meta">
              <strong>{{ row.book.title }}</strong>
              <span v-if="row.book.authors" class="finer subtle">{{ row.book.authors }}</span>
              <span v-for="reason in row.reasons" :key="reason" class="finer subtle">
                {{ suggestionReason(reason) }}
              </span>
            </span>
          </RouterLink>
          <button
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="nominating === row.book.id"
            @click="nominate(row)"
          >
            {{ t("common.nominate") }}
          </button>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.suggestions-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.suggestion-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.suggestion {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.suggestion-book {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: var(--space-3);
  color: inherit;
  text-decoration: none;
}

.suggestion-meta {
  display: grid;
  gap: 2px;
  align-content: start;
  min-width: 0;
}

.suggestion-meta strong {
  font-family: var(--serif);
  font-size: var(--text-md);
  line-height: var(--leading-snug);
}

@media (min-width: 1024px) {
  .vote-page {
    max-width: 860px;
  }
}
</style>
