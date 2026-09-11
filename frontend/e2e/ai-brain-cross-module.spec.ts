import { test, expect, gotoApp, TAB } from './fixtures';

test.describe('AI Brain Central Integration & Cross-Module Triggers', () => {
  test('navigating to AI assistant opens AI Brain from another module', async ({ app }) => {
    await gotoApp(app, TAB.todo);
    await app.getByRole('button', { name: TAB.ai }).click();

    // Verify lands on AI Assistant
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(app.getByText('How can I help you today?')).toBeVisible();
  });

  test('Tasks module "AI Prioritize" button invokes AI Brain with contextual prompt', async ({ app }) => {
    await gotoApp(app, TAB.todo);

    const aiPrioritizeBtn = app.getByRole('button', { name: 'AI Prioritize' });
    await expect(aiPrioritizeBtn).toBeVisible();
    await aiPrioritizeBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Analyze my pending tasks, deadlines, and priorities/i).first()
    ).toBeVisible();
  });

  test('Daily Log module "AI Reflection" button invokes AI Brain with contextual prompt', async ({ app }) => {
    await gotoApp(app, TAB.journal);

    const aiReflectionBtn = app.getByRole('button', { name: 'AI Reflection' });
    await expect(aiReflectionBtn).toBeVisible();
    await aiReflectionBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Generate a thoughtful reflection and summary of my activities/i).first()
    ).toBeVisible();
  });

  test('Habits module "AI Coach" button invokes AI Brain with contextual prompt', async ({ app }) => {
    await gotoApp(app, TAB.habits);

    const aiCoachBtn = app.getByRole('button', { name: 'AI Coach' });
    await expect(aiCoachBtn).toBeVisible();
    await aiCoachBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Review my habit consistency streaks and completion rates/i).first()
    ).toBeVisible();
  });

  test('Expenses module "AI Insights" button invokes AI Brain with contextual prompt', async ({ app }) => {
    await gotoApp(app, TAB.finance);

    const aiInsightsBtn = app.getByRole('button', { name: 'AI Insights' });
    await expect(aiInsightsBtn).toBeVisible();
    await aiInsightsBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Analyze my recent transactions and cashflow/i).first()
    ).toBeVisible();
  });

  test('Portfolio module "AI Review" button invokes AI Brain with contextual prompt', async ({ app }) => {
    await gotoApp(app, TAB.portfolio);

    const aiReviewBtn = app.getByRole('button', { name: 'AI Review' });
    await expect(aiReviewBtn).toBeVisible();
    await aiReviewBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Review my investment portfolio asset allocation, performance/i).first()
    ).toBeVisible();
  });

  test('Footprints module "AI Travel Log" button invokes AI Brain, clears input box, and does not show Sending', async ({ app }) => {
    await gotoApp(app, TAB.footprints);

    const aiTravelLogBtn = app.getByRole('button', { name: 'AI Travel Log' });
    await expect(aiTravelLogBtn).toBeVisible();
    await aiTravelLogBtn.click();

    // Verify AI Assistant view opens and receives prompt
    await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
    await expect(
      app.getByText(/Review my travel footprints and visited locations/i).first()
    ).toBeVisible();

    // Input box should NOT contain a copy of the message
    const input = app.getByPlaceholder(/Ask AI assistant/i);
    await expect(input).toHaveValue('');

    // Should not show "Sending..."
    await expect(app.getByText('Sending...')).not.toBeVisible();
  });
});
