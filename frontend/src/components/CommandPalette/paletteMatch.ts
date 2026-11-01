/**
 * Scores how well `query` matches `text`; higher is better, 0 means no match.
 * Tiers are spaced so a better tier always beats a worse one regardless of
 * length bonuses: exact > prefix > word prefix > substring > subsequence.
 */
export function scoreMatch(query: string, text: string): number {
  const q = query.trim().toLowerCase();
  const t = text.toLowerCase();
  if (q === '') return 1;
  if (t === q) return 1000;
  if (t.startsWith(q)) return 800 - Math.min(t.length - q.length, 99);
  if (t.split(/[\s\-_/]+/).some((word) => word.startsWith(q))) return 600 - Math.min(t.length, 99);
  const at = t.indexOf(q);
  if (at >= 0) return 400 - Math.min(at, 99);
  return isSubsequence(q, t) ? 100 : 0;
}

function isSubsequence(q: string, t: string): boolean {
  let i = 0;
  for (const ch of t) {
    if (ch === q[i]) i += 1;
    if (i === q.length) return true;
  }
  return false;
}

/** Filters to matches and orders them best-first; ties keep input order. */
export function rankByQuery<T>(query: string, items: readonly T[], haystack: (item: T) => string[]): T[] {
  return items
    .map((item, index) => ({
      item,
      index,
      score: Math.max(...haystack(item).map((text) => scoreMatch(query, text))),
    }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.item);
}
