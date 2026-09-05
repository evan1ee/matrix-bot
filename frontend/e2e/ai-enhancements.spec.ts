import { test, expect, gotoApp, TAB } from './fixtures';

test.describe('Smart AI Enhancements: Multi-Action Capture & Executive Retrospective', () => {
  test('executes multi-action natural language capture across finance, habits, and reminders', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.ai, exact: true }).click();
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();

    // Click the multi-action suggestion chip
    const multiPrompt = 'Spent $16.50 on lunch at Shake Shack, check off my workout habit, and remind me to call mom at 7pm';
    await app.getByRole('button', { name: multiPrompt }).click();

    // Verify user message in message list
    await expect(app.locator('.whitespace-pre-wrap', { hasText: multiPrompt })).toBeVisible();

    // Verify multi-action tool execution
    await expect(app.getByText(/Tool Result:/i)).toBeVisible();
    await expect(app.getByText('multi_action_executor')).toBeVisible();

    // Verify structured response covering all three distinct life domains
    await expect(app.getByText(/Recorded \$16\.50 for Dining at Shake Shack/i)).toBeVisible();
    await expect(app.getByText(/Marked Workout habit as completed/i)).toBeVisible();
    await expect(app.getByText(/Scheduled reminder to call mom at 7:00 PM/i)).toBeVisible();
  });

  test('opens Weekly Executive Retrospective modal and inspects consolidated metrics', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.ai, exact: true }).click();

    // Click "Weekly Retro" button in the header
    const retroBtn = app.getByRole('button', { name: 'Weekly Retro', exact: true });
    await expect(retroBtn).toBeVisible();
    await retroBtn.click();

    // Verify Retrospective Modal opens
    await expect(app.getByRole('heading', { name: 'Weekly Executive Retrospective' })).toBeVisible();
    await expect(app.getByText('2026-08-29 to 2026-09-05', { exact: true })).toBeVisible();

    // Verify stat cards
    await expect(app.getByText('USD 2374.50', { exact: true })).toBeVisible(); // Net cashflow
    await expect(app.getByText(/8 done/i)).toBeVisible(); // Completed tasks
    await expect(app.getByText('85.7%', { exact: true })).toBeVisible(); // Habit adherence
    await expect(app.getByText('good', { exact: true })).toBeVisible(); // Dominant mood

    // Verify copy summary button
    const copyBtn = app.getByRole('button', { name: /Copy Summary/i });
    await expect(copyBtn).toBeVisible();
    await copyBtn.click();
    await expect(app.getByText('Copied!')).toBeVisible();

    // Click "Ask AI to Plan Next Week"
    const planBtn = app.getByRole('button', { name: /Ask AI to Plan Next Week/i });
    await expect(planBtn).toBeVisible();
    await planBtn.click();

    // Modal should close and prompt should be sent to AI
    await expect(app.getByRole('heading', { name: 'Weekly Executive Retrospective' })).not.toBeVisible();
    await expect(app.getByText(/Please analyze my weekly retrospective and suggest 3 high-impact focus areas/i)).toBeVisible();
  });

  test('requests weekly retrospective in natural language conversation', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.ai, exact: true }).click();

    await app.getByPlaceholder(/Ask AI assistant/i).fill('Show me my weekly retrospective');
    await app.locator('form button[type="submit"]').click();

    // Verify AI response contains retrospective breakdown
    await expect(app.getByText(/Weekly Executive Retrospective/i)).toBeVisible();
    await expect(app.getByText(/Financial Velocity/i)).toBeVisible();
    await expect(app.getByText(/Productivity & Habits/i)).toBeVisible();
    await expect(app.getByText(/Habit Adherence/i)).toBeVisible();
  });
});
