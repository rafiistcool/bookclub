<script setup lang="ts">
import { useI18n } from "vue-i18n";
import QuoteList from "../components/QuoteList.vue";
import { useSession } from "../stores/session";

/** The reader's own saved lines, across books. Quotes are added on a book's page. */
const { t } = useI18n();
const session = useSession();
</script>

<template>
  <section class="quotes-page">
    <div class="page-head">
      <h1>{{ t("quotes.title") }}</h1>
      <p class="lede">{{ t("quotes.lede") }}</p>
    </div>
    <QuoteList
      v-if="session.user"
      :username="session.user.username"
      show-book
      :heading="t('quotes.yours')"
      :empty-text="t('quotes.emptyOwn')"
    />
  </section>
</template>
