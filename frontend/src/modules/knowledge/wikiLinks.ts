/** Longest title a wiki-link may carry; mirrors `MAX_TITLE_LEN` on the backend. */
const MAX_TITLE_LEN = 200;

const WIKI_LINK = /\[\[([^[\]\n|]+)(?:\|([^[\]\n]+))?\]\]/g;
// Fenced blocks and inline code: wiki-links inside them are literal text,
// matching the server's parser.
const CODE_SPAN = /(```[\s\S]*?```|~~~[\s\S]*?~~~|`[^`\n]*`)/;

const HREF_PREFIX = '#wiki:';

/** Case-insensitive key two titles are compared by. */
export function titleKey(title: string): string {
  return title.trim().toLowerCase();
}

/**
 * Rewrites `[[Title]]` / `[[Title|shown]]` as Markdown links to `#wiki:<title>`
 * so the preview renders them as clickable links. Code is left untouched.
 */
export function linkifyWikiLinks(markdown: string): string {
  return markdown
    .split(CODE_SPAN)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part.replace(WIKI_LINK, (_m, target: string, alias?: string) => {
            const title = target.trim();
            if (!title || title.length > MAX_TITLE_LEN) return _m;
            return `[${(alias ?? title).trim()}](${HREF_PREFIX}${encodeURIComponent(title)})`;
          })
    )
    .join('');
}

/** The title a rendered `#wiki:` link points at, or null for any other href. */
export function wikiLinkTitle(href: string | null): string | null {
  if (!href || !href.startsWith(HREF_PREFIX)) return null;
  try {
    return decodeURIComponent(href.slice(HREF_PREFIX.length));
  } catch {
    return null;
  }
}

export interface WikiQuery {
  /** Index of the opening `[[`. */
  start: number;
  /** What has been typed after it. */
  query: string;
}

/** The unfinished `[[query` the caret sits in, if any. */
export function activeWikiQuery(text: string, caret: number): WikiQuery | null {
  const before = text.slice(0, caret);
  const start = before.lastIndexOf('[[');
  if (start === -1) return null;
  const query = before.slice(start + 2);
  if (/[\]\n|[]/.test(query) || query.length > MAX_TITLE_LEN) return null;
  return { start, query };
}

/** Replaces the open `[[query` with a finished `[[title]]`; returns the new text and caret. */
export function completeWikiLink(
  text: string,
  { start }: WikiQuery,
  caret: number,
  title: string
): { text: string; caret: number } {
  const tail = text.slice(caret).replace(/^\]\]/, '');
  const inserted = `[[${title}]]`;
  return { text: text.slice(0, start) + inserted + tail, caret: start + inserted.length };
}
