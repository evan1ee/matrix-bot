import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';
import fs from 'fs';

const gotoMood = (page: import('@playwright/test').Page) => gotoApp(page, TAB.mood);

test.describe('Mood Module', () => {
  test('navigates dates, logs daily reflection with mood, weather, and notes', async ({ app, api }) => {
    await gotoMood(app);

    await expect(app.getByRole('heading', { name: 'Mood', exact: true })).toBeVisible();
    await expect(app.getByText('Today', { exact: true }).first()).toBeVisible();

    // Select Amazing mood
    await app.getByRole('button', { name: /Amazing/i }).click();

    // Select Sunny weather
    await app.getByRole('button', { name: /Sunny/i }).click();

    // Toggle Workout and Deep Work
    await app.getByRole('button', { name: /Workout/i }).click();
    await app.getByRole('button', { name: '💻 Deep Work' }).click();

    // Fill note
    await app.getByPlaceholder(/What happened today/i).fill('Shipped the new Mood module! Super focused.');

    // Save reflection
    await app.getByRole('button', { name: /save daily reflection/i }).click();

    // Verify it transitions to the Sealed view with Logged badge
    await expect(app.getByText('Logged', { exact: true }).first()).toBeVisible();
    await expect(app.getByText('Sealed Daily Reflection')).toBeVisible();
    await expect(app.getByText('Shipped the new Mood module! Super focused.')).toBeVisible();

    // Verify API state was updated
    const todayStr = new Date().toISOString().split('T')[0];
    const logged = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(logged).toBeDefined();
    expect(logged?.mood).toBe('amazing');
    expect(logged?.weather).toBe('sunny');
    expect(logged?.note).toContain('Shipped the new Mood module');
  });

  test('does not show auto-logged activity and displays sealed reflection notes and feel', async ({ app, api }) => {
    const todayStr = new Date().toISOString().split('T')[0];

    // Seed a logged reflection
    api.seedLifeDayLog({
      log_date: todayStr,
      mood: 'good',
      weather: 'cloudy',
      note: 'A quiet morning reflection.',
    });

    await gotoMood(app);

    // Verify Auto-logged Activity is NOT displayed anywhere in Mood module
    await expect(app.getByText('Auto-logged Activity')).not.toBeVisible();

    // Verify cross-navigation button to Time Blocks is NOT present
    await expect(app.getByRole('button', { name: /open time blocks/i })).not.toBeVisible();
    await expect(app.getByRole('button', { name: /time blocks →/i })).not.toBeVisible();

    // Verify sealed reflection is shown
    await expect(app.getByText('Sealed Daily Reflection')).toBeVisible();
    await expect(app.getByText('A quiet morning reflection.')).toBeVisible();
    await expect(app.getByRole('button', { name: /clear record/i })).toBeVisible();
  });

  test('clears a logged daily reflection', async ({ app, api }) => {
    const todayStr = new Date().toISOString().split('T')[0];
    api.seedLifeDayLog({
      log_date: todayStr,
      mood: 'good',
      weather: 'cloudy',
      note: 'A quiet and calm day',
    });

    await gotoMood(app);

    // Verify sealed reflection is shown
    await expect(app.getByText('Sealed Daily Reflection')).toBeVisible();
    await expect(app.getByText('A quiet and calm day')).toBeVisible();

    // Click Clear record
    await app.getByRole('button', { name: /clear record/i }).click();
    await expect(app.getByText(/Clear reflection record for/i)).toBeVisible();

    // Confirm clear
    await app.getByRole('button', { name: /yes, clear record/i }).click();

    // Should return to the unrecorded form
    await expect(app.getByText('Record Daily Reflection')).toBeVisible();

    // Verified removed from api
    const exists = api.lifeDayLogs.some((l) => l.log_date === todayStr);
    expect(exists).toBe(false);
  });

  test('navigates dates using stepping arrows, custom DatePicker popover, and Today jump button', async ({ app }) => {
    await gotoMood(app);

    const todayStr = new Date().toISOString().split('T')[0];
    const datePickerTrigger = app.locator('[data-testid="datepicker-container"] button');
    await expect(datePickerTrigger).toContainText(todayStr);

    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'mood_today_view.png') });
    }

    // When on today, "Back to today" is not shown
    await expect(app.getByRole('button', { name: /back to today/i })).not.toBeVisible();

    // Click Previous day arrow (<)
    await app.getByRole('button', { name: /previous day/i }).click();

    // Verify date changed to yesterday, not today
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    const yesterdayStr = yesterday.toISOString().split('T')[0];
    await expect(datePickerTrigger).toContainText(yesterdayStr);

    // "Back to today" button should now be visible when date is not today
    const backToTodayBtn = app.getByRole('button', { name: /back to today/i });
    await expect(backToTodayBtn).toBeVisible();

    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'mood_yesterday_with_back_to_today.png') });
    }

    // Click "Back to today" button
    await backToTodayBtn.click();
    await expect(datePickerTrigger).toContainText(todayStr);
    await expect(app.getByRole('button', { name: /back to today/i })).not.toBeVisible();

    // Click Next day arrow (>)
    await app.getByRole('button', { name: /next day/i }).click();
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    const tomorrowStr = tomorrow.toISOString().split('T')[0];
    await expect(datePickerTrigger).toContainText(tomorrowStr);
    await expect(app.getByRole('button', { name: /back to today/i })).toBeVisible();

    // Click DatePicker trigger to open custom popover
    await datePickerTrigger.click();
    await expect(app.locator('[data-testid="datepicker-popover"]')).toBeVisible();

    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'mood_datepicker_popover.png') });
    }

    // Pick Today from popover
    await app.locator('[data-testid="datepicker-popover"]').getByRole('button', { name: 'Today' }).click();
    await expect(app.locator('[data-testid="datepicker-popover"]')).not.toBeVisible();
    await expect(datePickerTrigger).toContainText(todayStr);
  });
});

