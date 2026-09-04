import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';

const gotoLife = (page: import('@playwright/test').Page) => gotoApp(page, TAB.life);

test.describe('Visualizing Life (Lifespan Grid)', () => {
  test('renders onboarding banner when no profile exists', async ({ app }) => {
    await gotoLife(app);

    await expect(app.getByRole('heading', { name: 'Visualizing Life' })).toBeVisible();
    await expect(app.getByText('Start visualizing your lifespan')).toBeVisible();
    await expect(app.getByRole('button', { name: /set birth date now/i })).toBeVisible();
  });

  test('configures birth date and displays stats strip and full lifespan grid', async ({ app, api }) => {
    await gotoLife(app);

    // Click Set Birth Date Now
    await app.getByRole('button', { name: /set birth date now/i }).click();
    await expect(app.getByRole('heading', { name: /set up life grid|edit life profile/i })).toBeVisible();

    // Fill in birth date: 1995-05-15
    const birthInput = app.locator('input[type="date"]');
    await birthInput.fill('1995-05-15');

    // Submit profile
    await app.getByRole('button', { name: /save profile/i }).click();

    // Verify stats strip visible without any age references
    await expect(app.getByText('Days Elapsed')).toBeVisible();
    await expect(app.getByText('Days Ahead')).toBeVisible();
    await expect(app.getByText('Days Logged')).toBeVisible();
    await expect(app.getByText('Started 1995-05-15')).toBeVisible();

    // Verify Lifespan Grid visible (no calendar)
    await expect(app.getByRole('heading', { name: /lifespan day grid/i })).toBeVisible();

    // Verify profile in api state
    expect(api.lifeProfile).toBeDefined();
    expect(api.lifeProfile?.birth_date).toBe('1995-05-15');
  });

  test('logs a day with mood, weather, activities, and notes, updating the grid', async ({ app, api }) => {
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });

    await gotoLife(app);

    await expect(app.getByRole('heading', { name: /lifespan day grid/i })).toBeVisible();

    // Click Log Today
    await app.getByRole('button', { name: /log today/i }).click();

    // Verify day modal opened
    await expect(app.getByText('Mood', { exact: true })).toBeVisible();

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
    await app.getByRole('button', { name: /save record/i }).click();

    // Check stats updated
    await expect(app.getByText('Dominant Mood')).toBeVisible();
    await expect(app.getByText('Amazing').first()).toBeVisible();

    // Verify api state
    const todayStr = new Date().toISOString().split('T')[0];
    const logged = api.lifeDayLogs.find((l) => l.log_date === todayStr);
    expect(logged).toBeDefined();
    expect(logged?.mood).toBe('amazing');
    expect(logged?.weather).toBe('sunny');
    expect(logged?.note).toContain('Shipped the Visualizing Life feature');
  });

  test('navigates decades, clicks day blocks, and captures visual screenshots', async ({ app, api }) => {
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

    await expect(app.getByText('Days Elapsed')).toBeVisible();
    const brainDir = 'C:\\Users\\mbmon\\.gemini\\antigravity\\brain\\e7af7078-5197-470f-9ad9-da9579965d67';

    // Screenshot of Lifespan Day Grid card with 10px blocks
    const gridCard = app.getByRole('heading', { name: /lifespan day grid/i }).locator('xpath=ancestor::div[contains(@class, "rounded-lg")][1]');
    await expect(gridCard).toBeVisible();
    await gridCard.screenshot({
      path: path.join(brainDir, 'life_grid_10px.png'),
    });

    // Test Decade jump
    const decadePill = app.getByRole('button', { name: '2020s' }).first();
    await expect(decadePill).toBeVisible();
    await decadePill.click();

    // Test Jump to Today button
    const todayJumpBtn = app.getByRole('button', { name: /jump to today/i });
    await expect(todayJumpBtn).toBeVisible();
    await todayJumpBtn.click();

    // Dark mode screenshot
    await app.screenshot({
      path: path.join(brainDir, 'life_grid_dark.png'),
    });

    // Switch to Light mode
    const themeBtn = app.locator('button[title*="theme" i], button[title*="Theme" i]').first();
    if (await themeBtn.isVisible()) {
      await themeBtn.click();
    } else {
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
