import { test, expect } from '@playwright/test';
import {
  isLinkableTitle,
  linkifyWikiLinks,
  wikiLinkTitle,
} from '../src/modules/knowledge/wikiLinks';

test('a title with parentheses survives the Markdown link round trip', () => {
  const md = linkifyWikiLinks("See [[Plan (v2)]] and [[It's *done*!]].");
  const hrefs = [...md.matchAll(/\]\((#wiki:[^)]*)\)/g)].map((m) => m[1]);
  expect(hrefs.map(wikiLinkTitle)).toEqual(['Plan (v2)', "It's *done*!"]);
});

test('titles that cannot be written as a link are not linkable', () => {
  expect(isLinkableTitle('Plain title')).toBe(true);
  for (const bad of ['a|b', 'a]b', 'a[b', 'a\nb', '   ', 'x'.repeat(201)]) {
    expect(isLinkableTitle(bad)).toBe(false);
  }
});
