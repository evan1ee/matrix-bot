import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';
import fs from 'fs';

const gotoJournal = (page: import('@playwright/test').Page) => gotoApp(page, TAB.journal);

test.describe('Daily Log (Journal Module)', () => {
  test('navigates dates, logs daily reflection with mood and notes', async ({ app, api }) => {
    await gotoJournal(app);

    await expect(app.getByRole('main').getByRole('heading', { name: /daily log/i })).toBeVisible();
    await expect(app.getByText('Today', { exact: true }).first()).toBeVisible();

    // Select Amazing mood
    await app.getByRole('button', { name: /Amazing/i }).click();

    // Select Sunny weather
    await app.getByRole('button', { name: /Sunny/i }).click();

    // Toggle Workout and Deep Work
    await app.getByRole('button', { name: /Workout/i }).click();
    await app.getByRole('button', { name: '💻 Deep Work' }).click();

    // Fill note
    await app.getByPlaceholder(/What happened today/i).fill('Shipped the new Daily Log module! Super focused.');

    // Save reflection
    await app.getByRole('button', { name: /save daily reflection/i }).click();

    // Verify it transitions to the Sealed view with Logged badge
    await expect(app.getByText('Logged', { exact: true })).toBeVisible();
    await expect(app.getByText('Sealed Daily Reflection')).toBeVisible();
    await expect(app.getByText('Shipped the new Daily Log module! Super focused.')).toBeVisible();

    // Verify API state was updated
    const todayStr = new Date().toISOString().split('T')[0];
    const logged = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(logged).toBeDefined();
    expect(logged?.mood).toBe('amazing');
    expect(logged?.weather).toBe('sunny');
    expect(logged?.note).toContain('Shipped the new Daily Log module');
  });

  test('auto-logs activities from todos, transactions, and habits in day summary feed', async ({ app, api }) => {
    const todayStr = new Date().toISOString().split('T')[0];

    // Seed a todo created today
    api.seedTodo({
      text: 'Deploy to production cluster',
      priority: 'high',
      completed: true,
      created_at: `${todayStr}T10:30:00Z`,
    });

    // Seed a transaction today
    api.seedTransaction({
      direction: 'expense',
      amount_minor: 4500,
      currency: 'USD',
      note: 'Team celebration dinner',
      occurred_at: `${todayStr}T19:00:00Z`,
    });

    // Seed a completed habit today
    const habit = api.seedHabit({ name: 'Evening Reading' });
    api.seedHabitLog({ habit_id: habit.id, log_date: todayStr, completed: true });

    // Open Daily Log
    await gotoJournal(app);

    // Verify auto-logged activity feed shows the cross-module actions
    await expect(app.getByText('Auto-logged Activity')).toBeVisible();
    await expect(app.getByText(/Deploy to production cluster/i).first()).toBeVisible();
    await expect(app.getByText(/Team celebration dinner/i)).toBeVisible();
    await expect(app.getByText(/Evening Reading/i)).toBeVisible();
    await expect(app.getByText(/-(USD|\$)45\.00/i)).toBeVisible();
  });

  test('allocates 24-hour time grid with presets and saves intraday blocks', async ({ app, api }) => {
    await gotoJournal(app);

    // Switch to 24h Time Grid tab
    await app.getByRole('button', { name: /24h time grid/i }).click();

    // Verify 24-Hour Time Allocation header
    await expect(app.getByText('24-Hour Time Allocation')).toBeVisible();
    await expect(app.getByText(/0\.0h \/ 24h tracked/i)).toBeVisible();

    // Paint a single block
    const block53 = app.locator('[data-block-idx="53"]');
    await expect(block53).toBeVisible();
    await block53.click();
    await expect(app.getByText(/0\.3h \/ 24h tracked/i)).toBeVisible();

    // Reset with Clear preset
    const clearPreset = app.getByRole('button', { name: /clear/i }).first();
    await clearPreset.click();
    await expect(app.getByText(/0\.0h \/ 24h tracked/i)).toBeVisible();

    // Use Sleep preset
    await app.getByRole('button', { name: /sleep/i }).first().click();
    await expect(app.getByText(/8\.0h \/ 24h tracked/i)).toBeVisible();

    // Use Workday preset
    await app.getByRole('button', { name: /workday/i }).first().click();
    await expect(app.getByText(/17\.0h \/ 24h tracked/i)).toBeVisible();

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

  test('clears a logged daily reflection', async ({ app, api }) => {
    const todayStr = new Date().toISOString().split('T')[0];
    api.seedLifeDayLog({
      log_date: todayStr,
      mood: 'good',
      weather: 'cloudy',
      note: 'A quiet and calm day',
    });

    await gotoJournal(app);

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
    await gotoJournal(app);

    const todayStr = new Date().toISOString().split('T')[0];
    const datePickerTrigger = app.locator('[data-testid="datepicker-container"] button');
    await expect(datePickerTrigger).toContainText(todayStr);

    const brainDir = process.env.BRAIN_DIR || '/Users/nosinx/.gemini/antigravity/brain/d97c3792-b421-4577-aa66-bc70c73a4412';
    if (fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'journal_today_view.png') });
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

    if (fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'journal_yesterday_with_back_to_today.png') });
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

    if (fs.existsSync(brainDir)) {
      await app.screenshot({ path: path.join(brainDir, 'journal_datepicker_popover.png') });
    }

    // Pick Today from popover
    await app.locator('[data-testid="datepicker-popover"]').getByRole('button', { name: 'Today' }).click();
    await expect(app.locator('[data-testid="datepicker-popover"]')).not.toBeVisible();
    await expect(datePickerTrigger).toContainText(todayStr);
  });
});
