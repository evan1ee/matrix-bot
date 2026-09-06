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
    await expect(app.getByText(multiPrompt)).toBeVisible();

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
    await expect(app.getByRole('heading', { name: 'Weekly Executive Retrospective', exact: true })).toBeVisible();
    await expect(app.getByText('2026-08-29 to 2026-09-05', { exact: true })).toBeVisible();

    // Verify stat cards
    await expect(app.getByText('USD 2374.50', { exact: true }).first()).toBeVisible(); // Net cashflow
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
    await expect(app.getByRole('heading', { name: 'Weekly Executive Retrospective', exact: true })).not.toBeVisible();
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

  test('renders multi-currency retrospective with isolated totals for USD and CNY without cross-currency summing', async ({ app }) => {
    await app.route('**/api/v1/ai/retrospective', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          period_start: '2026-08-30',
          period_end: '2026-09-06',
          total_spent_minor: 4500,
          total_income_minor: 0,
          net_cashflow_minor: -4500,
          currency: 'CNY',
          spends_by_currency: [
            { currency: 'CNY', currency_exponent: 2, spent_minor: 4500, income_minor: 0, net_minor: -4500 },
            { currency: 'USD', currency_exponent: 2, spent_minor: 1650, income_minor: 0, net_minor: -1650 },
          ],
          upcoming_bills_by_currency: [
            { currency: 'CNY', currency_exponent: 2, total_minor: 3000 },
            { currency: 'USD', currency_exponent: 2, total_minor: 1000 },
          ],
          formatted_spent: 'CNY 45.00, USD 16.50',
          formatted_income: 'CNY 0.00',
          formatted_net: 'CNY -45.00, USD -16.50',
          formatted_upcoming_bills: 'CNY 30.00, USD 10.00',
          top_expense_category: 'Groceries (CNY 45.00)',
          completed_tasks_count: 5,
          active_tasks_count: 2,
          habits_adherence_percent: 100.0,
          active_habits_count: 3,
          upcoming_bills_count: 2,
          upcoming_bills_total_minor: 3000,
          upcoming_reminders_count: 1,
          days_logged_count: 7,
          dominant_mood: 'great',
          formatted_markdown:
            '### 🌟 Weekly Executive Retrospective (2026-08-30 to 2026-09-06)\n\n' +
            '**💰 Financial Velocity**\n' +
            '- **Total Spent**: `CNY 45.00, USD 16.50`\n' +
            '- **Total Income**: `CNY 0.00`\n' +
            '- **Net Cashflow**: `CNY -45.00, USD -16.50`\n' +
            '- **Top Category**: Groceries (CNY 45.00)\n\n' +
            '**📅 Upcoming Focus (Next 7 Days)**\n' +
            '- **Due Bills**: 2 bills (`CNY 30.00, USD 10.00`)\n',
        }),
      });
    });

    await gotoApp(app);
    await app.getByRole('button', { name: TAB.ai, exact: true }).click();
    await app.getByRole('button', { name: 'Weekly Retro', exact: true }).click();

    // Verify modal heading
    await expect(app.getByRole('heading', { name: 'Weekly Executive Retrospective', exact: true })).toBeVisible();

    // Verify both currencies appear and 61.50 is NEVER displayed
    await expect(app.getByText('CNY -45.00', { exact: true })).toBeVisible();
    await expect(app.getByText(/Spent CNY 45\.00, USD 16\.50/)).toBeVisible();
    await expect(app.getByText('CNY 30.00, USD 10.00', { exact: true })).toBeVisible();
    await expect(app.getByText(/61\.50/)).not.toBeVisible();
  });
});
