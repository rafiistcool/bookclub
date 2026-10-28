<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import BookCover from "../../components/BookCover.vue";
import { bookPath, statusShort } from "../../constants";
import { isClubGone, useClubs, useCurrentClub } from "../../stores/clubs";
import { useToast } from "../../stores/toast";
import type { OverlapBook } from "../../types";

const { t } = useI18n();
const clubs = useClubs();
const toast = useToast();

const club = useCurrentClub();
const slug = computed(() => club.value.slug);
const overlap = ref<OverlapBook[]>([]);
const error = ref("");
const loaded = ref(false);
const includeReading = ref(false);
const nominating = ref<number | null>(null);

async function load() {
  loaded.value = false;
  try {
    overlap.value = (await api.overlap(slug.value, includeReading.value)).items;
    error.value = "";
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(slug.value);
    overlap.value = [];
    error.value = err instanceof ApiError ? err.message : t("club.overlapFailed");
  } finally {
    loaded.value = true;
  }
}

async function nominate(row: OverlapBook) {
  nominating.value = row.book.id;
  try {
    await api.nominate(slug.value, row.book.id);
    toast.show(t("shelf.nominated"));
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(slug.value);
    toast.show(err instanceof ApiError ? err.message : t("shelf.nominateFailed"));
  } finally {
    nominating.value = null;
  }
}

onMounted(load);
watch(includeReading, load);
</script>

<template>
  <section aria-labelledby="overlap">
    <div class="section-head">
      <h2 id="overlap">{{ t("club.overlap") }}</h2>
      <button
        class="chip"
        type="button"
        :aria-pressed="includeReading"
        :class="{ active: includeReading }"
        @click="includeReading = !includeReading"
      >
        {{ t("club.includeReading") }}
      </button>
    </div>
    <p class="fine muted overlap-blurb">
      {{ includeReading ? t("club.overlapBlurbReading") : t("club.overlapBlurb") }}
    </p>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton overlap-skeleton" aria-hidden="true" />

    <p v-else-if="overlap.length === 0" class="fine subtle">
      {{ t("club.noOverlap") }}
    </p>

    <ul v-else class="overlap-list">
      <li v-for="row in overlap" :key="row.book.id" class="overlap-row">
        <RouterLink class="overlap-book" :to="bookPath(row.book)">
          <BookCover :book="row.book" size="sm" />
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
                {{ member.username }} · {{ statusShort(member.status) }}
              </span>
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
</template>

<style scoped>
.overlap-skeleton {
  height: 130px;
}

.overlap-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.overlap-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
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
</style>
