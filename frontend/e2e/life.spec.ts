import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';
import fs from 'fs';

const gotoLife = (page: import('@playwright/test').Page) => gotoApp(page, TAB.life);

test.describe('Visualizing Life (Lifespan Grid)', () => {
  test('renders onboarding banner when no profile exists', async ({ app }) => {
    await gotoLife(app);

    await expect(app.getByRole('heading', { name: 'Life Grid' })).toBeVisible();
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

  test('navigates to Daily Log from header and inspects view-only day retrospective modal', async ({ app, api }) => {
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });

    await gotoLife(app);

    await expect(app.getByRole('heading', { name: /lifespan day grid/i })).toBeVisible();

    // Click Daily Log header button
    await app.getByRole('button', { name: /daily log →/i }).click();

    // Verify navigation to Daily Log module
    await expect(app.getByRole('heading', { name: 'Daily Log' })).toBeVisible();

    // Navigate back to Life Grid
    await gotoLife(app);
    await expect(app.getByRole('heading', { name: /lifespan day grid/i })).toBeVisible();

    // Open Today in Life Grid to inspect view-only retrospective modal
    await app.getByRole('button', { name: /open today/i }).click();

    // Verify view-only retrospective modal is shown
    await expect(app.getByText(/retrospective for this day/i)).toBeVisible();
    await expect(app.getByRole('button', { name: /open this day in daily log →/i })).toBeVisible();

    // Close modal
    await app.getByRole('button', { name: /close/i }).first().click();
    await expect(app.getByText(/retrospective for this day/i)).not.toBeVisible();
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
    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      // Screenshot of Lifespan Day Grid card with 10px blocks
      const gridCard = app.getByRole('heading', { name: /lifespan day grid/i }).locator('xpath=ancestor::div[contains(@class, "rounded-lg")][1]');
      await expect(gridCard).toBeVisible();
      await gridCard.screenshot({
        path: path.join(brainDir, 'life_grid_10px.png'),
      });
    }

    // Verify default decade filter matches current decade (2020s)
    await expect(app.getByRole('heading', { name: /lifespan day grid — 2020s/i })).toBeVisible();
    const decadePill = app.getByRole('button', { name: '2020s' }).first();
    await expect(decadePill).toBeVisible();
    await expect(decadePill).toHaveClass(/bg-neutral-900|bg-neutral-100/);
    await decadePill.click();

    // Test Open Today button
    const todayJumpBtn = app.getByRole('button', { name: /jump to today|open today/i });
    await expect(todayJumpBtn).toBeVisible();
    await todayJumpBtn.click();

    // Close modal opened by Open Today
    await expect(app.getByText(/retrospective for this day/i)).toBeVisible();
    await app.getByRole('button', { name: /close/i }).first().click();

    // Verify Lifespan Overview exists
    await expect(app.getByText('Lifespan overview')).toBeVisible();

    // Verify overview canvas does NOT open day modal on click
    const overviewCanvas = app.locator('canvas').first();
    const overviewBox = await overviewCanvas.boundingBox();
    if (overviewBox) {
      await app.mouse.click(overviewBox.x + overviewBox.width * 0.5, overviewBox.y + overviewBox.height * 0.5);
      await expect(app.getByText(/retrospective for this day/i)).not.toBeVisible();
    }

    // Hover over interactive grid canvas to activate the tooltip
    const canvas = app.locator('canvas').last();
    await expect(canvas).toBeVisible();
    await canvas.scrollIntoViewIfNeeded();
    const box = await canvas.boundingBox();
    if (box) {
      // Move mouse over canvas center
      await app.mouse.move(box.x + box.width * 0.45, box.y + box.height * 0.45);
      await app.waitForTimeout(300);

      // Verify the day HUD tooltip is visible
      await expect(app.getByText(/\d{4}-\d{2}-\d{2}/).first()).toBeVisible();

      if (fs.existsSync(brainDir)) {
        // Dark mode screenshot
        await app.screenshot({
          path: path.join(brainDir, 'life_grid_radial_dark.png'),
        });
        await app.screenshot({
          path: path.join(brainDir, 'life_grid_dark.png'),
        });
      }

      // Click on canvas to open day modal
      await app.mouse.click(box.x + box.width * 0.45, box.y + box.height * 0.45);
      await expect(app.getByText(/retrospective for this day/i)).toBeVisible();
      await app.getByRole('button', { name: /close/i }).first().click();
    }

    // Switch to Light mode
    const themeBtn = app.locator('button[title*="theme" i], button[title*="Theme" i]').first();
    if (await themeBtn.isVisible()) {
      await themeBtn.click();
    } else {
      await app.evaluate(() => {
        document.documentElement.classList.remove('dark');
      });
    }

    if (box && fs.existsSync(brainDir)) {
      // Hover near the far left edge to verify tooltip is never clipped
      await app.mouse.move(box.x + 25, box.y + 120);
      await app.waitForTimeout(300);

      await app.screenshot({
        path: path.join(brainDir, 'life_grid_left_edge.png'),
      });

      // Move mouse toward center for light mode screenshot
      await app.mouse.move(box.x + box.width * 0.35, box.y + box.height * 0.35);
      await app.waitForTimeout(300);

      // Light mode radial magnification screenshot
      await app.screenshot({
        path: path.join(brainDir, 'life_grid_radial_light.png'),
      });
      await app.screenshot({
        path: path.join(brainDir, 'life_grid_light.png'),
      });
    }
  });

  test('logged day is strictly view-only in life grid with navigation link to daily log', async ({ app, api }) => {
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });
    const todayStr = new Date().toISOString().split('T')[0];
    api.seedLifeDayLog({
      log_date: todayStr,
      mood: 'amazing',
      weather: 'sunny',
      activities: ['workout'],
      note: 'A memorable milestone day',
      time_allocation: JSON.stringify(['sleep', 'sleep', 'deep_work', 'deep_work']),
    });

    await gotoLife(app);

    // Click "Open today" button
    const todayBtn = app.getByRole('button', { name: /open today/i }).first();
    await expect(todayBtn).toBeVisible();
    await todayBtn.click();

    // Verify Read-Only View is shown with Logged badge
    await expect(app.getByRole('dialog').getByText('Logged', { exact: true })).toBeVisible();
    await expect(app.getByText(/Life Grid view-only retrospective for this day/i)).toBeVisible();
    await expect(app.getByText('A memorable milestone day')).toBeVisible();
    await expect(app.getByText('Sunny')).toBeVisible();
    await expect(app.getByText('Workout')).toBeVisible();

    // Verify 24h time allocation summary is rendered
    await expect(app.getByText('24h Time Allocation')).toBeVisible();
    await expect(app.getByText(/1\.0h tracked/i)).toBeVisible();

    // Verify editing controls are NOT visible in Life Grid
    await expect(app.getByRole('button', { name: /save record/i })).not.toBeVisible();
    await expect(app.getByRole('button', { name: /clear record/i })).not.toBeVisible();

    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({
        path: path.join(brainDir, 'life_logged_day_readonly.png'),
      });
    }

    // Click "Open this day in Daily Log →" to navigate to journal
    const navToJournalBtn = app.getByRole('button', { name: /open this day in daily log →/i });
    await expect(navToJournalBtn).toBeVisible();
    await navToJournalBtn.click();

    // Modal should close and Daily Log module should be opened
    await expect(app.getByText(/Life Grid view-only retrospective for this day/i)).not.toBeVisible();
    await expect(app.getByRole('heading', { name: 'Daily Log' })).toBeVisible();
    await expect(app.getByText('Sealed Daily Reflection')).toBeVisible();
    await expect(app.getByText('A memorable milestone day')).toBeVisible();
  });

  test('auto-logs activities from todos, transactions, and habits in view-only retrospective feed', async ({ app, api }) => {
    api.seedLifeProfile({ birth_date: '1995-05-15', expected_lifespan_years: 80 });

    const todayStr = new Date().toISOString().split('T')[0];

    // Seed a todo created today
    api.seedTodo({
      text: 'Deploy to production server',
      priority: 'high',
      completed: true,
      created_at: `${todayStr}T10:30:00Z`,
    });

    // Seed a transaction today
    api.seedTransaction({
      direction: 'expense',
      amount_minor: 3550,
      currency: 'USD',
      note: 'Team lunch catering',
      occurred_at: `${todayStr}T12:45:00Z`,
    });

    // Seed a completed habit today
    const habit = api.seedHabit({ name: 'Morning Meditation' });
    api.seedHabitLog({ habit_id: habit.id, log_date: todayStr, completed: true });

    await gotoLife(app);

    // Open Today retrospective modal
    await app.getByRole('button', { name: /open today/i }).click();

    // Verify auto-logged activity feed is visible
    await expect(app.getByText('Auto-logged Activity')).toBeVisible();
    await expect(app.getByText(/Deploy to production server/i).first()).toBeVisible();
    await expect(app.getByText(/Team lunch catering/i)).toBeVisible();
    await expect(app.getByText(/Morning Meditation/i)).toBeVisible();

    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      await app.screenshot({
        path: path.join(brainDir, 'life_day_modal_summary_tab.png'),
      });
    }
  });
});

