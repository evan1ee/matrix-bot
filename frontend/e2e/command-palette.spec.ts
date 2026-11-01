import { test, expect, gotoApp as gotoShell, DASHBOARD_READY } from './fixtures';
import type { Page } from '@playwright/test';

/** The shell renders before the session resolves; the palette only arms once signed in. */
async function gotoApp(app: Page) {
  await gotoShell(app);
  await expect(app.getByTitle(/Logged in as/).first()).toBeAttached();
}

const PALETTE = (app: Page) => app.getByRole('dialog', { name: 'Command palette' });

test.describe('command palette', () => {
  test('Ctrl+K opens it, Enter navigates to the matched page', async ({ app }) => {
    await gotoApp(app);
    await expect(app.getByText(DASHBOARD_READY)).toBeVisible();

    await app.keyboard.press('Control+k');
    await expect(PALETTE(app)).toBeVisible();
    await expect(app.getByRole('combobox')).toBeFocused();

    await app.getByRole('combobox').fill('expenses');
    await expect(app.getByRole('option', { name: /Expenses/ }).first()).toHaveAttribute('aria-selected', 'true');
    await app.keyboard.press('Enter');

    await expect(PALETTE(app)).toBeHidden();
    await expect(app.getByText('Record Entry')).toBeVisible();
  });

  test('Ctrl+K toggles it closed and Escape restores focus', async ({ app }) => {
    await gotoApp(app);
    const settings = app.getByRole('button', { name: 'Open command palette' });
    await settings.focus();

    await app.keyboard.press('Control+k');
    await expect(PALETTE(app)).toBeVisible();
    await app.keyboard.press('Escape');
    await expect(PALETTE(app)).toBeHidden();
    await expect(settings).toBeFocused();

    await app.keyboard.press('Control+k');
    await app.keyboard.press('Control+k');
    await expect(PALETTE(app)).toBeHidden();
  });

  test('arrow keys move the active option and Tab keeps focus in the input', async ({ app }) => {
    await gotoApp(app);
    await app.keyboard.press('Control+k');
    const input = app.getByRole('combobox');
    const options = app.getByRole('option');

    await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
    await app.keyboard.press('ArrowDown');
    await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
    await expect(input).toHaveAttribute('aria-activedescendant', (await options.nth(1).getAttribute('id')) ?? '');
    await app.keyboard.press('ArrowUp');
    await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');

    await app.keyboard.press('Tab');
    await expect(input).toBeFocused();
  });

  test('a quick action jumps to the module with its input focused', async ({ app }) => {
    await gotoApp(app);
    await app.keyboard.press('Control+k');
    await app.getByRole('combobox').fill('create task');
    await app.keyboard.press('Enter');

    await expect(app.getByText('Create New Task')).toBeVisible();
    await expect(app.locator('main input:focus, main textarea:focus').first()).toBeVisible();
  });

  test('typing searches notes after the debounce and opens the Notes page', async ({ app, api }) => {
    api.seedNote({ title: 'Quarterly zebra plan', content: 'Stripes everywhere' });
    const searches: string[] = [];
    app.on('request', (req) => {
      if (req.url().endsWith('/knowledge/search')) searches.push(req.postData() ?? '');
    });
    await gotoApp(app);

    await app.keyboard.press('Control+k');
    await app.getByRole('combobox').pressSequentially('zebra', { delay: 20 });
    const hit = app.getByRole('option', { name: /Quarterly zebra plan/ });
    await expect(hit).toBeVisible();
    // One debounced request for the settled text, not one per keystroke.
    expect(searches.length).toBe(1);
    expect(searches[0]).toContain('zebra');

    await hit.click();
    await expect(PALETTE(app)).toBeHidden();
    await expect(app.getByText('Quarterly zebra plan')).toBeVisible();
  });

  test('the theme action flips the theme', async ({ app }) => {
    await gotoApp(app);
    const isDark = () => app.evaluate(() => document.documentElement.classList.contains('dark'));
    const before = await isDark();

    await app.keyboard.press('Control+k');
    await app.getByRole('combobox').fill('theme');
    await app.keyboard.press('Enter');

    await expect.poll(isDark).toBe(!before);
  });

  test('does not open on the signed-out screen', async ({ anonApp }) => {
    await anonApp.goto('/');
    await anonApp.keyboard.press('Control+k');
    await anonApp.waitForTimeout(300);
    await expect(PALETTE(anonApp)).toBeHidden();
  });
});
