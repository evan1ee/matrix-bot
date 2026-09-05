import { test as base, expect, Locator, Page } from '@playwright/test';
import { FakeApi } from './fake-api';

/**
 * A label that only appears once the shell has rendered its dashboard.
 *
 * Named rather than inlined because every spec waits on it: when the sidebar
 * redesign renamed this stat from "Total Tasks", the old string was left in
 * eight files and the whole suite failed on the first assertion.
 */
export const DASHBOARD_READY = 'Active Tasks';

/**
 * Sidebar labels, as rendered.
 *
 * Note these are *not* the modules' own `navTab.label` values — `Sidebar.tsx`
 * hardcodes its own list, so `todo` renders as "Tasks" while its registry entry
 * says "Todos", and `finance` renders as "Expenses" against "Finance". Two
 * sources of truth for one label; the specs follow the one a user actually
 * sees. Worth collapsing in the app, and this map is the single place the specs
 * would need to change when that happens.
 */
export const TAB = {
  dashboard: 'Dashboard',
  todo: 'Tasks',
  reminder: 'Reminders',
  bookmarks: 'Bookmarks',
  habits: 'Habits',
  finance: 'Expenses',
  portfolio: 'Portfolio',
  bills: 'Bills',
  footprints: 'Footprints',
  life: 'Life Grid',
  webhooks: 'Webhooks',
  ai: 'AI Assistant',
} as const;

interface Fixtures {
  api: FakeApi;
  /** A page with the fake API installed and a session already established. */
  app: Page;
  /** A page with the fake API installed but signed out. */
  anonApp: Page;
}

export const test = base.extend<Fixtures>({
  api: async ({}, use) => {
    await use(new FakeApi());
  },

  anonApp: async ({ page, api }, use) => {
    api.signedIn = false;
    await api.install(page);
    await use(page);
  },

  // Signed in. There is nothing to seed on the client any more — the session
  // is an HttpOnly cookie the page cannot read or write, so "signed in" is
  // decided entirely by whether `/auth/me` answers. `FakeApi.signedIn` is what
  // controls that.
  app: async ({ page, api }, use) => {
    api.signedIn = true;
    await api.install(page);
    await use(page);
  },
});

export { expect };

/** Opens the app and waits for the shell to be interactive. */
export async function gotoApp(page: Page, moduleTab?: string) {
  await page.goto('/');
  await expect(page.getByText(DASHBOARD_READY)).toBeVisible();
  if (moduleTab) {
    await page.getByRole('button', { name: moduleTab, exact: true }).click();
  }
}

/**
 * Formats a future instant as the `YYYY-MM-DDTHH:mm` the date picker exchanges.
 * The suite pins the browser to UTC, so this needs no zone maths.
 */
export function datetimeLocal(offsetMinutes: number): string {
  const d = new Date(Date.now() + offsetMinutes * 60_000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}T${pad(
    d.getUTCHours()
  )}:${pad(d.getUTCMinutes())}`;
}

/** The read-only row for a task, located by its text. */
export function todoRow(page: Page, text: string): Locator {
  return page.getByTestId('todo-row').filter({ hasText: text });
}

/** The expanded edit form for a task. */
export function todoEditForm(page: Page): Locator {
  return page.getByTestId('todo-row-edit');
}

/**
 * Picks an option from a `CustomSelect`.
 *
 * It is a button plus a popover rather than a `<select>`, so there is nothing
 * for `selectOption` to act on — open it, then click the option inside the same
 * container.
 */
export async function chooseOption(trigger: Locator, optionLabel: string) {
  await trigger.click();
  await trigger.locator('..').getByRole('button', { name: optionLabel, exact: true }).click();
}

/**
 * Picks a currency from a `CustomSelect` populated from `CURRENCY_OPTIONS`.
 *
 * Not `chooseOption`: `CURRENCY_OPTIONS` labels are `"EUR — Euro"`, not the
 * bare code, so an exact match on the code alone never finds the option —
 * match by prefix instead.
 */
export async function chooseCurrency(scope: Locator | Page, code: string) {
  const trigger = selectField(scope, 'Currency');
  await trigger.click();
  await trigger.locator('..').getByRole('button', { name: new RegExp(`^${code}\\b`) }).click();
}

/** The `CustomSelect` trigger inside the field labelled `fieldLabel`. */
export function selectField(scope: Locator | Page, fieldLabel: string): Locator {
  return scope.locator('label').filter({ hasText: fieldLabel }).first().getByRole('button').first();
}

/** The `DateTimePicker` inside the field labelled `fieldLabel`. */
export function dateField(scope: Locator | Page, fieldLabel: string): Locator {
  return scope
    .locator('label')
    .filter({ hasText: fieldLabel })
    .first()
    .getByTestId('datetime-picker');
}

/**
 * Drives a `DateTimePicker` to an absolute local instant, stated as the
 * `YYYY-MM-DDTHH:mm` `datetimeLocal` produces.
 *
 * The control is a calendar popover, not a native input, so there is no value
 * to `fill` — walk to the month, click the day, then set the two time selects.
 */
export async function setDateTime(picker: Locator, value: string) {
  const [datePart, timePart] = value.split('T');
  const [hour, minute] = timePart.split(':');
  const targetMonth = datePart.slice(0, 7);

  await picker.getByRole('button').first().click();
  // Scoped to the page, not the picker: the popover renders through
  // `createPortal`, so it is not a descendant of the control that opened it and
  // a picker-scoped locator can never match. Only one is ever open at a time.
  const popover = picker.page().getByTestId('datetime-popover');
  const monthLabel = popover.getByTestId('datetime-month');

  // Stepped rather than matched against the header text, which is rendered in
  // the viewer's locale. `data-month` sorts lexicographically as `YYYY-MM`.
  for (let i = 0; i < 36; i++) {
    const shown = await monthLabel.getAttribute('data-month');
    if (shown === targetMonth) break;
    await popover
      .getByRole('button', { name: shown! < targetMonth ? 'Next month' : 'Previous month' })
      .click();
  }

  await popover.locator(`[data-day="${datePart}"]`).click();
  await pickTimeUnit(popover.getByTestId('datetime-hour'), hour);
  await pickTimeUnit(popover.getByTestId('datetime-minute'), minute);
  await popover.getByRole('button', { name: 'Done' }).click();
}

/**
 * Sets one of the picker's time selects. Not `chooseOption`: the trigger shows
 * the current value, so asking for the value it already holds would match both
 * the trigger and its option — the option is always the later of the two.
 */
async function pickTimeUnit(field: Locator, label: string) {
  await field.getByRole('button').first().click();
  await field.getByRole('button', { name: label, exact: true }).last().click();
}

/** Empties a `DateTimePicker`. */
export async function clearDateTime(picker: Locator) {
  await picker.getByRole('button').first().click();
  await picker.page().getByTestId('datetime-popover').getByRole('button', { name: 'Clear' }).click();
}
