import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';
import fs from 'fs';

const gotoTimeBlocks = (page: import('@playwright/test').Page) => gotoApp(page, TAB.time_blocks);

test.describe('Time Blocks Module', () => {
  test('allocates 24-hour time grid with presets and saves intraday blocks', async ({ app, api }) => {
    await gotoTimeBlocks(app);

    await expect(app.getByRole('heading', { name: 'Time Blocks', exact: true })).toBeVisible();
    await expect(app.getByText('24-Hour Time Allocation')).toBeVisible();
    await expect(app.getByText(/0\.0h \/ 24h tracked/i)).toBeVisible();

    // Paint a single block
    const block53 = app.locator('[data-block-idx="53"]');
    await expect(block53).toBeVisible();
    await block53.click();
    await expect(app.getByText(/0\.3h \/ 24h tracked/i).first()).toBeVisible();

    // Reset with Clear preset
    const clearPreset = app.getByRole('button', { name: /clear/i }).first();
    await clearPreset.click();
    await expect(app.getByText(/0\.0h \/ 24h tracked/i).first()).toBeVisible();

    // Use Sleep preset
    await app.getByRole('button', { name: /sleep/i }).first().click();
    await expect(app.getByText(/8\.0h \/ 24h tracked/i).first()).toBeVisible();

    // Use Workday preset
    await app.getByRole('button', { name: /workday/i }).first().click();
    await expect(app.getByText(/17\.0h \/ 24h tracked/i).first()).toBeVisible();

    // Save time blocks
    const saveBtn = app.getByRole('button', { name: /save time blocks/i });
    await saveBtn.click();
    await expect(app.getByText(/✓ Time blocks saved|✓ Saved/i).first()).toBeVisible();

    // Verify API state
    const todayStr = new Date().toISOString().split('T')[0];
    const savedLog = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(savedLog).toBeDefined();
    expect(savedLog?.time_allocation).toContain('deep_work');
    expect(savedLog?.time_allocation).toContain('sleep');
  });

  test('navigates dates using stepping arrows and custom DatePicker popover', async ({ app }) => {
    await gotoTimeBlocks(app);

    const todayStr = new Date().toISOString().split('T')[0];
    const datePickerTrigger = app.locator('[data-testid="datepicker-container"] button');
    await expect(datePickerTrigger).toContainText(todayStr);

    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'time_blocks_today_view.png') });
    }

    // Click Previous day arrow (<)
    await app.getByRole('button', { name: /previous day/i }).click();

    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    const yesterdayStr = yesterday.toISOString().split('T')[0];
    await expect(datePickerTrigger).toContainText(yesterdayStr);

    // "Back to today" button is visible
    const backToTodayBtn = app.getByRole('button', { name: /back to today/i });
    await expect(backToTodayBtn).toBeVisible();
    await backToTodayBtn.click();
    await expect(datePickerTrigger).toContainText(todayStr);

    // Click DatePicker popover
    await datePickerTrigger.click();
    await expect(app.locator('[data-testid="datepicker-popover"]')).toBeVisible();

    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'time_blocks_datepicker_popover.png') });
    }

    await app.locator('[data-testid="datepicker-popover"]').getByRole('button', { name: 'Today' }).click();
    await expect(app.locator('[data-testid="datepicker-popover"]')).not.toBeVisible();
    await expect(datePickerTrigger).toContainText(todayStr);
  });

  test('preserves existing mood and reflections when saving time allocation', async ({ app, api }) => {
    const todayStr = new Date().toISOString().split('T')[0];

    // Pre-seed a day reflection in Mood
    api.seedLifeDayLog({
      log_date: todayStr,
      mood: 'amazing',
      weather: 'sunny',
      note: 'Deeply joyful morning reflection.',
    });

    await gotoTimeBlocks(app);

    // Apply sleep and workday presets and save
    await app.getByRole('button', { name: /sleep/i }).first().click();
    await app.getByRole('button', { name: /workday/i }).first().click();
    await app.getByRole('button', { name: /save time blocks/i }).click();
    await expect(app.getByText(/✓ Time blocks saved|✓ Saved/i).first()).toBeVisible();

    // Verify both mood and time allocation are present in the API
    const log = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(log).toBeDefined();
    expect(log?.mood).toBe('amazing');
    expect(log?.weather).toBe('sunny');
    expect(log?.note).toBe('Deeply joyful morning reflection.');
    expect(log?.time_allocation).toContain('deep_work');
    expect(log?.time_allocation).toContain('sleep');
  });
});
