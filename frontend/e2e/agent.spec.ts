import { test, expect, gotoApp, TAB } from './fixtures';

test.describe('Autonomous Agent Module', () => {
  test('navigates to agent module and lists active & completed runs', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Verify sub-tabs and controls are visible
    await expect(app.getByRole('button', { name: 'Agent Runs', exact: true })).toBeVisible();
    await expect(app.getByRole('button', { name: 'Scheduled Tasks', exact: true })).toBeVisible();
    await expect(app.getByRole('button', { name: /start run/i })).toBeVisible();

    // Verify seeded runs are rendered in the table
    await expect(app.getByText('Audit monthly expenses and categorize unassigned items')).toBeVisible();
    await expect(app.getByText('Deposit funds and buy AAPL shares')).toBeVisible();
    await expect(app.getByRole('table').getByText('Completed')).toBeVisible();
    await expect(app.getByText('Approval Needed')).toBeVisible();
  });

  test('filters agent runs by status and search query', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Filter by Completed
    await app.getByRole('button', { name: 'Completed', exact: true }).click();
    await expect(app.getByText('Audit monthly expenses and categorize unassigned items')).toBeVisible();
    await expect(app.getByText('Deposit funds and buy AAPL shares')).not.toBeVisible();

    // Filter by Approvals
    await app.getByRole('button', { name: 'Approvals', exact: true }).click();
    await expect(app.getByText('Deposit funds and buy AAPL shares')).toBeVisible();
    await expect(app.getByText('Audit monthly expenses and categorize unassigned items')).not.toBeVisible();

    // Reset to All
    await app.getByRole('button', { name: 'All', exact: true }).click();
    await expect(app.getByText('Audit monthly expenses and categorize unassigned items')).toBeVisible();
    await expect(app.getByText('Deposit funds and buy AAPL shares')).toBeVisible();

    // Filter by Search Query
    const searchInput = app.locator('input[placeholder*="Search goals"]');
    await searchInput.fill('audit monthly');
    await expect(app.getByText('Audit monthly expenses and categorize unassigned items')).toBeVisible();
    await expect(app.getByText('Deposit funds and buy AAPL shares')).not.toBeVisible();

    await searchInput.fill('');
    await expect(app.getByText('Deposit funds and buy AAPL shares')).toBeVisible();
  });

  test('opens run detail drawer and inspects execution trace and thoughts', async ({ app }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Click on the completed run to view details
    await app.getByText('Audit monthly expenses and categorize unassigned items').click();

    // Verify drawer contents
    await expect(app.getByRole('heading', { name: 'Run Details', exact: true })).toBeVisible();
    await expect(app.getByText('Goal Objective')).toBeVisible();
    await expect(app.getByText('Steps Progress')).toBeVisible();

    // Verify step timeline entries
    await expect(app.getByText('query_transactions')).toBeVisible();
    await expect(app.getByText('categorize_transaction')).toBeVisible();
    await expect(app.getByText('Synthesizing response')).toBeVisible();

    // Toggle thought accordion
    const thoughtBtn = app.getByRole('button', { name: /thought reasoning/i }).first();
    await expect(thoughtBtn).toBeVisible();
    await thoughtBtn.click();
    await expect(app.getByText(/fetch recent transactions from the finance service/i)).not.toBeVisible();
    // Click again to re-expand
    await thoughtBtn.click();
    await expect(app.getByText(/fetch recent transactions from the finance service/i)).toBeVisible();

    // Close drawer via close button
    const closeBtn = app.locator('button[aria-label="Close"], button:has(svg.lucide-x)').last();
    await closeBtn.click();
    await expect(app.getByRole('heading', { name: 'Run Details', exact: true })).not.toBeVisible();
  });

  test('dispatches a new autonomous agent goal', async ({ app, api }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Open start run modal
    await app.getByRole('button', { name: /start run/i }).click();
    await expect(app.getByRole('heading', { name: 'Start Autonomous Agent Goal' })).toBeVisible();

    // Fill in goal prompt
    const goalText = 'Reconcile unreconciled bank deposits and notify in Ops room';
    await app.locator('#agent-goal').fill(goalText);

    // Submit goal
    await app.getByRole('button', { name: 'Start Goal' }).click();

    // Modal closes and run appears in list & drawer
    await expect(app.getByRole('heading', { name: 'Start Autonomous Agent Goal' })).not.toBeVisible();
    await expect(app.getByRole('table').getByText(goalText)).toBeVisible();
    await expect(app.getByRole('dialog').getByText(goalText)).toBeVisible();

    // Assert request sent to fake API
    const postReq = api.requests.find((r) => r.method === 'POST' && r.url.endsWith('/agent/runs'));
    expect(postReq).toBeDefined();
    expect((postReq?.body as any)?.goal).toBe(goalText);
  });

  test('reviews and approves pending sensitive tool authorization', async ({ app, api }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Sensitive approval banner is visible
    await expect(app.getByText(/1 action requires approval/i)).toBeVisible();
    await expect(app.getByRole('button', { name: /review action/i })).toBeVisible();

    // Open review modal
    await app.getByRole('button', { name: /review action/i }).click();
    await expect(app.getByRole('heading', { name: 'Authorize Sensitive Agent Action' })).toBeVisible();
    await expect(app.getByText('execute_portfolio_deposit')).toBeVisible();

    // Click Approve Action
    await app.getByRole('button', { name: 'Approve Action' }).click();

    // Modal closes and approval banner disappears
    await expect(app.getByRole('heading', { name: 'Authorize Sensitive Agent Action' })).not.toBeVisible();
    await expect(app.getByText(/1 action requires approval/i)).not.toBeVisible();

    // Verify approve request sent to fake API
    const approveReq = api.requests.find(
      (r) => r.method === 'POST' && r.url.includes('/agent/approvals/appr-1/approve')
    );
    expect(approveReq).toBeDefined();
  });

  test('views and modifies recurring scheduled tasks', async ({ app, api }) => {
    await gotoApp(app);
    await app.getByRole('button', { name: TAB.agent, exact: true }).click();

    // Switch to Scheduled Tasks tab
    await app.getByRole('button', { name: 'Scheduled Tasks', exact: true }).click();
    await expect(app.getByRole('heading', { name: 'Registered Autonomous Workflows' })).toBeVisible();

    // Verify seeded scheduled tasks
    await expect(app.getByText('WeeklyBookmarkDigestTask')).toBeVisible();
    await expect(app.getByText('MonthlyFinancialAuditTask')).toBeVisible();
    await expect(app.getByText('0 9 * * 1')).toBeVisible();

    // Open task configuration modal
    await app.getByRole('button', { name: 'Configure...' }).first().click();
    await expect(app.getByRole('heading', { name: 'Configure Scheduled Task' })).toBeVisible();

    // Update cron expression
    const cronInput = app.locator('#task-cron');
    await cronInput.fill('0 10 * * 1');

    // Save changes
    await app.getByRole('button', { name: 'Save Changes' }).click();
    await expect(app.getByRole('heading', { name: 'Configure Scheduled Task' })).not.toBeVisible();

    // Updated cron expression appears
    await expect(app.getByText('0 10 * * 1')).toBeVisible();

    // Verify update request sent to fake API
    const updateReq = api.requests.find(
      (r) => r.method === 'PUT' && r.url.includes('/agent/tasks/WeeklyBookmarkDigestTask')
    );
    expect(updateReq).toBeDefined();
    expect((updateReq?.body as any)?.cron_expression).toBe('0 10 * * 1');
  });
});
