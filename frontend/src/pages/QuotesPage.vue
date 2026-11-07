<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import QuoteList from "../components/QuoteList.vue";
import { useSession } from "../stores/session";

/** The reader's own saved lines, across books. Quotes are added on a book's page. */
const { t } = useI18n();
const session = useSession();
/** The list's count, shown beside the title instead of under a second heading. */
const count = ref<number | null>(null);
</script>

<template>
  <section class="quotes-page">
    <div class="page-head">
      <div class="title-row">
        <h1>{{ t("quotes.title") }}</h1>
        <span v-if="count" class="fine subtle nums quotes-count">{{ count }}</span>
      </div>
    </div>
    <QuoteList
      v-if="session.user"
      :username="session.user.username"
      show-book
      heading-hidden
      :heading="t('quotes.yours')"
      :empty-text="t('quotes.emptyOwn')"
      @changed="count = $event"
    />
  </section>
</template>

<style scoped>
.title-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
}
</style>
