<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { ApiError } from "../api/client";
import { isTermsChanged } from "../legal";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";

/**
 * The one-time prompt to accept the operator's terms (`UserOut.needs_terms`):
 * accounts imported from the old app, sign-ups from before the terms, and
 * everyone after a new terms version. It stays until the reader accepts;
 * the server records the version and the time (`POST /api/auth/terms`).
 */
const { t } = useI18n();
const service = useService();
const session = useSession();
const toast = useToast();

const pending = ref(false);
const error = ref("");

async function accept() {
  if (pending.value) return;
  error.value = "";
  pending.value = true;
  try {
    if (!service.terms_version) await service.reloadLegal();
    const version = service.terms_version;
    if (!version) throw new Error("no terms version");
    await session.acceptTerms(version);
    toast.show(t("terms.accepted"));
  } catch (err) {
    // The terms changed while this page was open: the next tap accepts the new ones.
    if (isTermsChanged(err)) await service.reloadLegal();
    error.value = err instanceof ApiError ? err.message : t("terms.failed");
  } finally {
    pending.value = false;
  }
}
</script>

<template>
  <section v-if="session.user?.needs_terms" class="panel terms-prompt" aria-labelledby="terms-prompt">
    <h2 id="terms-prompt">{{ t("terms.promptTitle") }}</h2>
    <p class="fine muted">
      <i18n-t keypath="terms.promptBody" tag="span" scope="global">
        <template #terms>
          <RouterLink to="/terms">{{ t("legal.inline.terms") }}</RouterLink>
        </template>
        <template #name>{{ service.name }}</template>
      </i18n-t>
      <template v-if="service.hasLegal('privacy')">
        {{ " " }}
        <i18n-t keypath="legal.privacyNote" tag="span" scope="global">
          <template #privacy>
            <RouterLink to="/privacy">{{ t("legal.inline.privacy") }}</RouterLink>
          </template>
        </i18n-t>
      </template>
    </p>
    <p v-if="error" class="error fine">{{ error }}</p>
    <div class="btn-row">
      <button class="btn btn-primary btn-sm" type="button" :disabled="pending" @click="accept">
        {{ pending ? t("terms.accepting") : t("terms.accept") }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.terms-prompt {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-5);
  border-color: var(--accent-line);
}

.terms-prompt h2 {
  font-size: var(--text-lg);
}
</style>
