import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';

const gotoHabits = (page: import('@playwright/test').Page) => gotoApp(page, TAB.habits);

test.describe('Daily Cadence & Heatmap Habits', () => {
  test('renders empty habits view with stats, heatmap, and add prompt', async ({ app }) => {
    await gotoHabits(app);

    await expect(app.getByRole('heading', { name: /Daily Cadence Heatmap/i })).toBeVisible();
    await expect(app.getByRole('heading', { name: /Habits & Rituals/i })).toBeVisible();

    await expect(app.getByText('Active Habits', { exact: true })).toBeVisible();
    await expect(app.getByText("Today's Cadence")).toBeVisible();
    await expect(app.getByText('Best Streak')).toBeVisible();
    await expect(app.getByText('Total Check-ins')).toBeVisible();

    await expect(app.getByText('No habits found')).toBeVisible();
    await expect(app.getByTestId('add-habit-button')).toBeVisible();
  });

  test('creates a new habit and displays it with initial streak and category', async ({ app, api }) => {
    await gotoHabits(app);

    await app.getByTestId('add-habit-button').click();
    await expect(app.getByRole('heading', { name: 'New Habit' })).toBeVisible();

    await app.getByPlaceholder(/Morning Run/i).fill('Daily Meditation');
    await app.getByPlaceholder(/Minimum 3 miles/i).fill('15 minutes mindful breathing');

    const brainDir = 'C:\\Users\\mbmon\\.gemini\\antigravity\\brain\\e7af7078-5197-470f-9ad9-da9579965d67';
    await app.screenshot({
      path: path.join(brainDir, 'habits_modal_dark.png'),
    });

    await app.getByRole('button', { name: 'Create Habit' }).click();

    const row = app.getByTestId(/^habit-row-\d+/).first();
    await expect(row.getByText('Daily Meditation')).toBeVisible();
    await expect(row.getByText('15 minutes mindful breathing')).toBeVisible();
    await expect(row.getByText('wellness')).toBeVisible();

    const created = api.habits.find((h) => h.name === 'Daily Meditation');
    expect(created).toBeDefined();
    expect(created?.category).toBe('wellness');
  });

  test('toggles habit completion for today, advancing streak and today\'s cadence', async ({ app, api }) => {
    api.seedHabit({
      name: 'Read 30 mins',
      category: 'learning',
      description: 'Technical books',
    });

    await gotoHabits(app);

    await expect(app.getByText('Read 30 mins')).toBeVisible();
    await expect(app.getByText('0 / 1')).toBeVisible();

    // Toggle completed
    const row = app.getByTestId(/^habit-row-\d+/).first();
    await row.getByRole('button', { name: /Check off/i }).click();

    // Today's cadence should update to 1 / 1
    await expect(app.getByText('1 / 1')).toBeVisible();
    await expect(app.getByText('100% completed')).toBeVisible();

    // Verify streak updated
    await expect(row.getByText('1d')).toBeVisible();
  });

  test('filters habits by category tabs and search query', async ({ app, api }) => {
    api.seedHabit({ name: 'Morning Yoga', category: 'fitness' });
    api.seedHabit({ name: 'Deep Work Session', category: 'productivity' });
    api.seedHabit({ name: 'Cold Shower', category: 'wellness' });

    await gotoHabits(app);

    await expect(app.getByText('Morning Yoga')).toBeVisible();
    await expect(app.getByText('Deep Work Session')).toBeVisible();
    await expect(app.getByText('Cold Shower')).toBeVisible();

    // Filter by category fitness
    await app.getByTestId('category-tab-fitness').click();
    await expect(app.getByText('Morning Yoga')).toBeVisible();
    await expect(app.getByText('Deep Work Session')).not.toBeVisible();
    await expect(app.getByText('Cold Shower')).not.toBeVisible();

    // Return to all
    await app.getByTestId('category-tab-all').click();
    await expect(app.getByText('Deep Work Session')).toBeVisible();

    // Search query
    await app.getByPlaceholder('Search habits...').fill('Cold');
    await expect(app.getByText('Cold Shower')).toBeVisible();
    await expect(app.getByText('Morning Yoga')).not.toBeVisible();
  });

  test('deletes a habit', async ({ app, api }) => {
    api.seedHabit({ name: 'Obsolete Habit' });

    await gotoHabits(app);
    await expect(app.getByText('Obsolete Habit')).toBeVisible();

    app.on('dialog', (dialog) => dialog.accept());
    await app.getByRole('button', { name: /Delete Obsolete Habit/i }).click();

    await expect(app.getByText('No habits found')).toBeVisible();
    expect(api.habits.length).toBe(0);
  });

  test('captures screenshots in light and dark mode for visual verification', async ({ app, api }) => {
    api.seedHabit({ name: 'Morning Run 5km', category: 'fitness', description: 'Zone 2 cardio' });
    api.seedHabit({ name: 'Deep Work 90m', category: 'productivity', description: 'Zero notifications' });
    api.seedHabit({ name: 'Evening Reading', category: 'learning', description: 'Read 25 pages' });

    const todayStr = new Date().toISOString().split('T')[0];
    api.seedHabitLog({ habit_id: 1, log_date: todayStr, completed: true });
    api.seedHabitLog({ habit_id: 2, log_date: todayStr, completed: true });

    // Seed some heatmap data over past weeks
    for (let i = 1; i <= 30; i++) {
      const d = new Date();
      d.setDate(d.getDate() - i);
      const ds = d.toISOString().split('T')[0];
      if (i % 2 === 0) {
        api.seedHabitLog({ habit_id: 1, log_date: ds, completed: true });
      }
      if (i % 3 === 0) {
        api.seedHabitLog({ habit_id: 2, log_date: ds, completed: true });
      }
    }

    await gotoHabits(app);
    await expect(app.getByText('Morning Run 5km')).toBeVisible();

    const brainDir = 'C:\\Users\\mbmon\\.gemini\\antigravity\\brain\\e7af7078-5197-470f-9ad9-da9579965d67';

    // Dark theme screenshot
    await app.screenshot({
      path: path.join(brainDir, 'habits_dashboard_dark.png'),
      fullPage: true,
    });

    // Light theme
    await app.locator('button[title*="light mode" i]:visible, button[title*="dark mode" i]:visible').first().click();
    await app.waitForTimeout(300);

    await app.screenshot({
      path: path.join(brainDir, 'habits_dashboard_light.png'),
      fullPage: true,
    });
  });
});
