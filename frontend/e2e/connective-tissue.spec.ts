import { test, expect, gotoApp, TAB } from './fixtures';

test.describe('Connective Tissue: Automated Ledger Postings', () => {
  test('paying a recurring bill automatically records an expense in finance ledger with bill source badge', async ({
    app,
    api,
  }) => {
    // 1. Seed a recurring bill
    const bill = api.seedRecurringBill({
      name: 'Spotify Premium',
      category: 'subscription',
      amount_minor: 1099,
      currency: 'USD',
      frequency: 'monthly',
      next_due_at: new Date(Date.now() + 86400000).toISOString(),
    });

    // 2. Navigate to Bills module
    await gotoApp(app, TAB.bills);
    await expect(app.getByText('Spotify Premium')).toBeVisible();

    // 3. Mark the bill as paid
    const billRow = app.locator('tr').filter({ hasText: 'Spotify Premium' });
    await billRow.getByRole('button', { name: 'Pay' }).click();

    // Verify paid installments incremented in fake API state
    expect(bill.paid_installments).toBe(1);

    // 4. Navigate to Expenses (Finance) module
    await gotoApp(app, TAB.finance);

    // 5. Verify the posted transaction row appears in the finance ledger
    const txRow = app.getByTestId('ledger-row').filter({ hasText: 'Recurring bill: Spotify Premium' });
    await expect(txRow).toBeVisible();
    await expect(txRow).toContainText('10.99');
    await expect(txRow.getByTestId('badge-source-bill')).toBeVisible();

    // 6. Verify the transaction model in API has source_kind and source_id set
    const tx = api.transactions.find((t) => t.note === 'Recurring bill: Spotify Premium');
    expect(tx).toBeDefined();
    expect(tx?.source_kind).toBe('recurring_bill');
    expect(tx?.source_id).toBe(bill.id);
    expect(tx?.direction).toBe('expense');
    expect(tx?.amount_minor).toBe(1099);
  });

  test('depositing cash in portfolio automatically posts to finance ledger with portfolio badge', async ({
    app,
    api,
  }) => {
    // 1. Navigate to Portfolio module
    await gotoApp(app, TAB.portfolio);

    // 2. Open Cash Management and deposit 500 USD
    const panel = app.getByTestId('cash-panel');
    await panel.getByPlaceholder('500.00').fill('500.00');
    await panel.getByRole('button', { name: 'Deposit' }).click();

    // 3. Navigate to Expenses (Finance) module
    await gotoApp(app, TAB.finance);

    // 4. Verify transaction row appears in the finance ledger
    const txRow = app.getByTestId('ledger-row').filter({ hasText: 'Portfolio cash deposit (USD)' });
    await expect(txRow).toBeVisible();
    await expect(txRow).toContainText('500.00');
    await expect(txRow.getByTestId('badge-source-portfolio')).toBeVisible();

    // 5. Verify transaction model in API
    const tx = api.transactions.find((t) => t.note === 'Portfolio cash deposit (USD)');
    expect(tx).toBeDefined();
    expect(tx?.source_kind).toBe('portfolio_cash');
    expect(tx?.direction).toBe('expense');
    expect(tx?.amount_minor).toBe(50000);
  });

  test('withdrawing cash from portfolio automatically posts income to finance ledger', async ({
    app,
    api,
  }) => {
    // Seed initial cash balance of 1000 USD
    api.seedCash('USD', 100000);

    // 1. Navigate to Portfolio module
    await gotoApp(app, TAB.portfolio);

    // 2. Withdraw 250 USD from cash panel
    const panel = app.getByTestId('cash-panel');
    await panel.getByPlaceholder('500.00').fill('250.00');
    await panel.getByRole('button', { name: 'Withdraw' }).click();

    // 3. Navigate to Expenses (Finance) module
    await gotoApp(app, TAB.finance);

    // 4. Verify income transaction row appears in the finance ledger
    const txRow = app.getByTestId('ledger-row').filter({ hasText: 'Portfolio cash withdrawal (USD)' });
    await expect(txRow).toBeVisible();
    await expect(txRow).toContainText('250.00');
    await expect(txRow.getByTestId('badge-source-portfolio')).toBeVisible();

    // 5. Verify transaction model in API
    const tx = api.transactions.find((t) => t.note === 'Portfolio cash withdrawal (USD)');
    expect(tx).toBeDefined();
    expect(tx?.source_kind).toBe('portfolio_cash');
    expect(tx?.direction).toBe('income');
    expect(tx?.amount_minor).toBe(25000);
  });

  test('editing a zero-decimal (JPY) recurring bill preserves integer minor units', async ({
    app,
    api,
  }) => {
    const bill = api.seedRecurringBill({
      name: 'Tokyo Gym',
      category: 'subscription',
      amount_minor: 8000,
      currency: 'JPY',
      currency_exponent: 0,
      frequency: 'monthly',
      next_due_at: '2026-11-15 09:00:00',
    });

    await gotoApp(app, TAB.bills);
    const billRow = app.locator('tr').filter({ hasText: 'Tokyo Gym' });
    await expect(billRow).toContainText('JPY 8000');

    await billRow.getByTitle('Edit Bill').click();
    await expect(app.getByRole('heading', { name: 'Edit Recurring Bill' })).toBeVisible();
    const amountInput = app.getByPlaceholder('0');
    await expect(amountInput).toHaveValue('8000');

    await amountInput.fill('9500');
    await app.getByRole('button', { name: 'Save Changes' }).click();

    await expect(billRow).toContainText('JPY 9500');
    expect(bill.amount_minor).toBe(9500);
  });
});
