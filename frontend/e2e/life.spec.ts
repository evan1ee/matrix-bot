import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';

const gotoLife = (page: import('@playwright/test').Page) => gotoApp(page, TAB.life);

test.describe('Visualizing Life (Life Grid)', () => {
  test('renders onboarding banner when no profile exists', async ({ app }) => {
    await gotoLife(app);

    await expect(app.getByRole('heading', { name: 'Visualizing Life' })).toBeVisible();
    await expect(app.getByText('Start Visualizing Your Lifespan')).toBeVisible();
    await expect(app.getByRole('button', { name: 'Set Birth Date Now' })).toBeVisible();
  });

  test('configures birth date and displays stats strip and canvas minimap', async ({ app, api }) => {
    await gotoLife(app);

    // Click Set Birth Date Now
    await app.getByRole('button', { name: 'Set Birth Date Now' }).click();
    await expect(app.getByRole('heading', { name: 'Set Up Life Grid' })).toBeVisible();

    // Fill in birth date: 1995-05-15
    const birthInput = app.locator('input[type="date"]');
    await birthInput.fill('1995-05-15');

    // Submit profile
    await app.getByRole('button', { name: 'Start Visualizing Life' }).click();

    // Verify stats strip visible
    await expect(app.getByText('Age in Days')).toBeVisible();
    await expect(app.getByText('Life Lived')).toBeVisible();
    await expect(app.getByText('Days Logged')).toBeVisible();
    await expect(app.getByText('Born 1995-05-15')).toBeVisible();

    // Verify Life Horizon canvas visible
    await expect(app.getByRole('heading', { name: 'Life Horizon' })).toBeVisible();

    // Verify profile in api state
    expect(api.lifeProfile).toBeDefined();
    expect(api.lifeProfile?.birth_date).toBe('1995-05-15');
  });

  test('logs a day with mood, weather, activities, and notes, updating the grid', async ({ app, api }) => {
    // Seed profile and some initial logs
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });

    await gotoLife(app);

    // Check year 2026 header and age
    await expect(app.getByText('2026', { exact: true })).toBeVisible();
    await expect(app.getByText('Age 31')).toBeVisible();

    // Click Log Today
    await app.getByRole('button', { name: 'Log Today' }).click();

    // Verify day modal opened
    await expect(app.getByText('How was your day? (Mood)')).toBeVisible();

    // Select Amazing mood
    await app.getByRole('button', { name: /Amazing/i }).click();

    // Select Sunny weather
    await app.getByRole('button', { name: /Sunny/i }).click();

    // Toggle Workout and Deep Work
    await app.getByRole('button', { name: /Workout/i }).click();
    await app.getByRole('button', { name: /Deep Work/i }).click();

    // Fill note
    await app.getByPlaceholder(/What happened today/i).fill('Shipped the Visualizing Life feature! Feel great.');

    const brainDir = 'C:\\Users\\mbmon\\.gemini\\antigravity\\brain\\e7af7078-5197-470f-9ad9-da9579965d67';
    await app.screenshot({
      path: path.join(brainDir, 'life_day_modal_dark.png'),
    });

    // Save record
    await app.getByRole('button', { name: 'Save Record' }).click();

    // Check stats updated
    await expect(app.getByText('Dominant Mood')).toBeVisible();
    await expect(app.getByText('Amazing')).toBeVisible();

    // Verify api state
    const todayStr = new Date().toISOString().split('T')[0];
    const logged = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(logged).toBeDefined();
    expect(logged?.mood).toBe('amazing');
    expect(logged?.weather).toBe('sunny');
    expect(logged?.note).toContain('Shipped the Visualizing Life feature');
  });

  test('navigates years and captures visual screenshots', async ({ app, api }) => {
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });

    // Seed several day logs with different moods
    api.seedLifeDayLog({ log_date: '2026-01-01', mood: 'amazing', note: 'New Year Party' });
    api.seedLifeDayLog({ log_date: '2026-02-14', mood: 'good', note: 'Valentine Dinner' });
    api.seedLifeDayLog({ log_date: '2026-04-10', mood: 'neutral', note: 'Regular workday' });
    api.seedLifeDayLog({ log_date: '2026-06-01', mood: 'amazing', note: 'Summer start' });
    api.seedLifeDayLog({ log_date: '2026-08-20', mood: 'good', note: 'Mountain hike' });
    api.seedLifeDayLog({
      log_date: new Date().toISOString().split('T')[0],
      mood: 'amazing',
      note: 'Present day',
    });

    await gotoLife(app);

    await expect(app.getByText('Age in Days')).toBeVisible();
    await expect(app.getByText('Life Horizon')).toBeVisible();

    const brainDir = 'C:\\Users\\mbmon\\.gemini\\antigravity\\brain\\e7af7078-5197-470f-9ad9-da9579965d67';

    // Dark mode screenshot
    await app.screenshot({
      path: path.join(brainDir, 'life_grid_dark.png'),
    });

    // Switch to Light mode
    const themeBtn = app.locator('button[title*="theme" i], button[title*="Theme" i]').first();
    if (await themeBtn.isVisible()) {
      await themeBtn.click();
    } else {
      // Toggle via document element class
      await app.evaluate(() => {
        document.documentElement.classList.remove('dark');
      });
    }

    await app.waitForTimeout(500);

    // Light mode screenshot
    await app.screenshot({
      path: path.join(brainDir, 'life_grid_light.png'),
    });
  });
});
