<script lang="ts">
/*
 * Sanitising the operator's HTML.
 *
 * The texts come from the operator's own files (`LEGAL_DIR`), exported by a
 * generator, and the page renders them with `v-html`. They are trusted to be
 * the operator's, not to be harmless: a generator template, a pasted snippet
 * or a tampered file must not run script in the app's origin, where the
 * session cookie works. So the HTML passes an allow-list first:
 *
 * - Only text elements stay (headings, paragraphs, lists, tables, links,
 *   emphasis, sections). Script, style, frames, embeds, forms, pictures and
 *   foreign content (SVG, MathML) go with everything inside them. Pictures
 *   go too, so that the page never loads anything from a third party.
 * - Other elements (`font`, `center`, custom elements) are unwrapped: their
 *   text stays, the element does not.
 * - Only a few attributes stay (`id`, `lang`, `dir`, `title`, `href` on
 *   links, table spans, the placeholder marker). Event handlers, `style`,
 *   `class` and `target` never do.
 * - A link keeps its `href` only for http, https, mailto and tel. Links to
 *   other sites open in a new tab without an opener.
 * - Comments go.
 *
 * The HTML is parsed with `DOMParser`, whose document runs no script and
 * loads nothing. The serialised result contains no raw-text element, so the
 * browser parses it back into the same tree.
 */

/** Removed together with everything inside them. */
const DROP = new Set([
  "applet",
  "audio",
  "base",
  "button",
  "canvas",
  "datalist",
  "dialog",
  "embed",
  "form",
  "frame",
  "frameset",
  "head",
  "iframe",
  "img",
  "input",
  "link",
  "map",
  "math",
  "meta",
  "noembed",
  "noframes",
  "noscript",
  "object",
  "option",
  "param",
  "picture",
  "portal",
  "script",
  "select",
  "slot",
  "source",
  "style",
  "svg",
  "template",
  "textarea",
  "title",
  "track",
  "video",
]);

/**
 * Kept, with these attributes besides the global ones. A `Map`, not an object:
 * `tag in {…}` would also match `Object.prototype` names such as
 * `constructor`, which the HTML parser accepts as a tag name.
 */
const KEEP: ReadonlyMap<string, readonly string[]> = new Map(
  Object.entries({
    a: ["href"],
    abbr: [],
    address: [],
    article: [],
    aside: [],
    b: [],
    blockquote: [],
    br: [],
    caption: [],
    cite: [],
    code: [],
    col: ["span"],
    colgroup: ["span"],
    dd: [],
    del: [],
    dfn: [],
    div: [],
    dl: [],
    dt: [],
    em: [],
    figcaption: [],
    figure: [],
    footer: [],
    h1: [],
    h2: [],
    h3: [],
    h4: [],
    h5: [],
    h6: [],
    header: [],
    hr: [],
    i: [],
    ins: [],
    kbd: [],
    li: ["value"],
    mark: [],
    ol: ["start", "reversed", "type"],
    p: [],
    pre: [],
    q: [],
    s: [],
    section: [],
    small: [],
    span: [],
    strong: [],
    sub: [],
    sup: [],
    table: [],
    tbody: [],
    td: ["colspan", "rowspan"],
    tfoot: [],
    th: ["colspan", "rowspan", "scope"],
    thead: [],
    time: ["datetime"],
    tr: [],
    u: [],
    ul: [],
  }),
);

/** Allowed on every kept element. `data-legal-placeholder` marks the development texts. */
const GLOBAL_ATTRIBUTES = new Set(["id", "lang", "dir", "title", "data-legal-placeholder"]);

const LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

/** The base relative links resolve against while checking them; never shown. */
const CHECK_BASE = "https://legal.invalid/";

/** The scheme an `href` resolves to, the way the browser reads it (tabs, newlines and spaces ignored). */
function linkProtocol(href: string): string | null {
  try {
    return new URL(href, CHECK_BASE).protocol;
  } catch {
    return null;
  }
}

/** A link to another site: an absolute http(s) URL on another origin. */
function isExternal(href: string, origin: string): boolean {
  try {
    const url = new URL(href, CHECK_BASE);
    if (url.origin === new URL(CHECK_BASE).origin) return false;
    return (url.protocol === "http:" || url.protocol === "https:") && url.origin !== origin;
  } catch {
    return false;
  }
}

function cleanAttributes(element: Element, tag: string, origin: string): void {
  const allowed = KEEP.get(tag) ?? [];
  for (const { name } of Array.from(element.attributes)) {
    if (!GLOBAL_ATTRIBUTES.has(name) && !allowed.includes(name)) element.removeAttribute(name);
  }
  if (tag !== "a") return;
  const href = element.getAttribute("href");
  if (href === null) return;
  const protocol = linkProtocol(href);
  if (!protocol || !LINK_PROTOCOLS.has(protocol)) {
    element.removeAttribute("href");
    return;
  }
  if (isExternal(href, origin)) {
    element.setAttribute("target", "_blank");
    element.setAttribute("rel", "noopener noreferrer");
  }
}

function cleanChildren(parent: Node, origin: string): void {
  for (const node of Array.from(parent.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) continue;
    if (node.nodeType !== Node.ELEMENT_NODE) {
      parent.removeChild(node);
      continue;
    }
    const element = node as Element;
    // HTML elements only: anything in another namespace is foreign content.
    const tag = element.namespaceURI === "http://www.w3.org/1999/xhtml" ? element.localName : "";
    if (!tag || DROP.has(tag)) {
      parent.removeChild(element);
      continue;
    }
    cleanChildren(element, origin);
    if (KEEP.has(tag)) {
      cleanAttributes(element, tag, origin);
    } else {
      // Unknown or structural (`html`, `body`, `font`, `center`, custom elements): keep the content only.
      while (element.firstChild) parent.insertBefore(element.firstChild, element);
      parent.removeChild(element);
    }
  }
}

/**
 * The operator's legal HTML with everything but text, structure and safe
 * links removed (see above). `origin` is the app's own, whose links stay in
 * the tab.
 */
export function sanitizeLegalHtml(html: string, origin = globalThis.location?.origin ?? ""): string {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  const body = parsed.body;
  cleanChildren(body, origin);
  return body.innerHTML;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import BackLink from "../components/BackLink.vue";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import LegalLinks from "../components/LegalLinks.vue";
import { formatDay } from "../i18n/dates";
import { useLocale } from "../stores/locale";
import { useService } from "../stores/service";
import type { LegalDoc, LegalDocumentOut } from "../types";

/**
 * `/imprint`, `/privacy` and `/terms`: the operator's text from
 * `GET /api/legal/{doc}`, in the reader's language or, when the operator has
 * only the other one, in that language with a note. Public, signed in or out,
 * outside the app shell.
 *
 * The HTML passes `sanitizeLegalHtml` (the module script above) before
 * `v-html`: text and safe links only, no script, handlers, styles or pictures.
 */
const props = defineProps<{ doc: LegalDoc }>();

const { t } = useI18n();
const router = useRouter();
const service = useService();
const locale = useLocale();

const state = ref<"loading" | "ready" | "missing" | "failed">("loading");
const text = ref<LegalDocumentOut | null>(null);
const article = ref<HTMLElement | null>(null);
const html = computed(() => (text.value ? sanitizeLegalHtml(text.value.html) : ""));
const updated = computed(() => (text.value?.updated ? formatDay(text.value.updated, "UTC", locale.locale) : ""));
let latest = 0;

async function load() {
  const request = ++latest;
  // Another language of the same text: keep showing this one until it arrives.
  if (text.value?.doc !== props.doc) state.value = "loading";
  try {
    const answer = await api.legal(props.doc, locale.locale);
    if (request !== latest) return;
    text.value = answer;
    state.value = "ready";
  } catch (err) {
    if (request !== latest) return;
    text.value = null;
    state.value = err instanceof ApiError && err.status === 404 ? "missing" : "failed";
  }
}

watch(() => [props.doc, locale.locale], load, { immediate: true });

/**
 * Links inside the text: `#section` scrolls within it, a path of this app
 * opens through the router, and anything else (other sites, mail) is left to
 * the browser.
 */
function onTextClick(event: MouseEvent) {
  if (event.defaultPrevented || event.button !== 0) return;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const target = event.target instanceof Element ? event.target : null;
  const link = target?.closest("a[href]");
  if (!(link instanceof HTMLAnchorElement) || link.target) return;
  const href = link.getAttribute("href") ?? "";
  if (href.startsWith("#")) {
    event.preventDefault();
    let id = href.slice(1);
    try {
      id = decodeURIComponent(id);
    } catch {
      /* Keep it as written. */
    }
    const section = id ? document.getElementById(id) : null;
    if (section && article.value?.contains(section)) section.scrollIntoView();
    return;
  }
  const url = new URL(link.href);
  if (url.origin !== window.location.origin || /^\/(api|media)(\/|$)/.test(url.pathname)) return;
  event.preventDefault();
  void router.push(url.pathname + url.search + url.hash);
}
</script>

<template>
  <main class="legal-page" :aria-busy="state === 'loading'">
    <header class="legal-head">
      <RouterLink class="wordmark" to="/">{{ service.name }}</RouterLink>
      <LanguageSwitch compact />
    </header>
    <BackLink :label="t('legal.back')" fallback="/" />

    <div v-if="state === 'loading'" class="legal-skeleton" aria-hidden="true">
      <div class="skeleton skeleton-title" />
      <div class="skeleton skeleton-line" />
      <div class="skeleton skeleton-line" />
      <div class="skeleton skeleton-line short" />
    </div>

    <template v-else-if="state === 'ready' && text">
      <p v-if="text.fallback" class="fine muted legal-note">{{ t(`legal.onlyIn.${text.locale}`) }}</p>
      <article
        ref="article"
        class="legal-text"
        :lang="text.locale"
        :aria-label="t(`legal.title.${doc}`)"
        @click="onTextClick"
        v-html="html"
      />
      <p v-if="updated" class="fine subtle legal-updated">{{ t("legal.updated", { date: updated }) }}</p>
    </template>

    <template v-else>
      <h1 class="legal-title">{{ t(`legal.title.${doc}`) }}</h1>
      <template v-if="state === 'failed'">
        <p class="error">{{ t("legal.failed") }}</p>
        <div class="btn-row">
          <button class="btn btn-ghost btn-sm" type="button" @click="load">{{ t("common.tryAgain") }}</button>
        </div>
      </template>
      <p v-else class="lede">{{ t("legal.missing") }}</p>
      <p v-if="service.contact_email" class="fine muted legal-contact">
        <i18n-t keypath="legal.contact" tag="span" scope="global">
          <template #email>
            <a :href="`mailto:${service.contact_email}`">{{ service.contact_email }}</a>
          </template>
        </i18n-t>
      </p>
    </template>

    <footer class="legal-foot">
      <LegalLinks />
    </footer>
  </main>
</template>

<style scoped>
.legal-page {
  max-width: 760px;
  min-height: 100dvh;
  margin: 0 auto;
  padding: calc(var(--space-6) + env(safe-area-inset-top)) var(--space-5)
    calc(var(--space-7) + env(safe-area-inset-bottom));
  display: flex;
  flex-direction: column;
}

.legal-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

.legal-skeleton {
  display: grid;
  gap: var(--space-3);
  margin-top: var(--space-5);
}

.skeleton-title {
  height: 32px;
  width: 50%;
}

.skeleton-line {
  height: 14px;
}

.skeleton-line.short {
  width: 60%;
}

.legal-note {
  margin: var(--space-3) 0;
}

.legal-title {
  margin: var(--space-5) 0 var(--space-3);
}

.legal-page .error,
.legal-page .lede {
  margin-bottom: var(--space-4);
}

.legal-contact {
  overflow-wrap: anywhere;
}

.legal-updated {
  margin-top: var(--space-5);
}

.legal-foot {
  margin-top: auto;
  padding-top: var(--space-7);
}

/* The operator's HTML: headings, paragraphs, lists and tables, in the app's type. */
.legal-text {
  max-width: 70ch;
  margin-top: var(--space-4);
  overflow-wrap: anywhere;
}

.legal-text :deep(:is(h1, h2, h3, h4, h5, h6)) {
  margin: var(--space-6) 0 var(--space-2);
}

.legal-text :deep(:is(h1, h2, h3, h4, h5, h6):first-child) {
  margin-top: 0;
}

.legal-text :deep(:is(h4, h5, h6)) {
  font-size: var(--text-md);
}

.legal-text :deep(:is(p, ul, ol, dl, table, blockquote, pre, address, figure)) {
  margin: 0 0 var(--space-3);
}

.legal-text :deep(:is(p, ul, ol, dl, table, blockquote, pre, address, figure):last-child) {
  margin-bottom: 0;
}

.legal-text :deep(:is(ul, ol)) {
  padding-left: var(--space-5);
}

.legal-text :deep(address) {
  font-style: normal;
}

.legal-text :deep(blockquote) {
  padding-left: var(--space-4);
  border-left: 3px solid var(--border);
}

.legal-text :deep(table) {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  border-collapse: collapse;
}

.legal-text :deep(:is(th, td)) {
  padding: var(--space-1) var(--space-2);
  border: 1px solid var(--border);
  text-align: left;
  vertical-align: top;
}

.legal-text :deep(hr) {
  height: 1px;
  border: 0;
  background: var(--border);
  margin: var(--space-5) 0;
}

/* The development placeholders (server/legal-placeholder/) say what they are. */
.legal-text :deep([data-legal-placeholder]) {
  margin-bottom: var(--space-5);
  padding: var(--space-3) var(--space-4);
  border: 1px dashed var(--danger-border);
  border-radius: var(--radius-md);
  background: var(--surface);
}
</style>
