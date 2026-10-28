import type { LegalDoc } from "./types";

/**
 * The legal texts (#53): the operator's imprint, privacy policy and terms of
 * use, served by `GET /api/legal/{doc}` as HTML fragments, and the terms the
 * reader accepts.
 */

/** Every document, in the order the links list them. */
export const LEGAL_DOCS: readonly LegalDoc[] = ["imprint", "privacy", "terms"];

/** The page of a document. Each one is public, signed in or out. */
export function legalPath(doc: LegalDoc): string {
  return `/${doc}`;
}

/**
 * The server's 400 when a sign-up or `POST /api/auth/terms` sent a terms
 * version that is no longer the current one (`msg::TERMS_CHANGED`).
 */
export const TERMS_CHANGED = "The terms of use have changed. Reload the page and try again.";

/** True for the 400 that says the terms changed since the page loaded them. */
export function isTermsChanged(error: unknown): boolean {
  return (
    error !== null &&
    typeof error === "object" &&
    (error as { status?: unknown }).status === 400 &&
    (error as { detail?: unknown }).detail === TERMS_CHANGED
  );
}

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
