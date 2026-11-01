import { test, expect } from '@playwright/test';
import { rankByQuery, scoreMatch } from '../src/components/CommandPalette/paletteMatch';

test('tiers rank exact above prefix above word prefix above substring above subsequence', () => {
  expect(scoreMatch('fin', 'fin')).toBeGreaterThan(scoreMatch('fin', 'finance'));
  expect(scoreMatch('fin', 'finance')).toBeGreaterThan(scoreMatch('fin', 'time finance'));
  expect(scoreMatch('fin', 'time finance')).toBeGreaterThan(scoreMatch('fin', 'refinance'));
  expect(scoreMatch('fin', 'refinance')).toBeGreaterThan(scoreMatch('fn', 'finance'));
  expect(scoreMatch('zzz', 'finance')).toBe(0);
});

test('empty query keeps every item in input order', () => {
  expect(rankByQuery('', ['b', 'a', 'c'], (s) => [s])).toEqual(['b', 'a', 'c']);
});

test('ranking drops non-matches, is case-insensitive and stable on ties', () => {
  const items = ['Expenses', 'Bills', 'Reminders', 'Notes'];
  expect(rankByQuery('E', items, (s) => [s])).toEqual(['Expenses', 'Reminders', 'Notes']);
  expect(rankByQuery('xp', items, (s) => [s])).toEqual(['Expenses']);
});
