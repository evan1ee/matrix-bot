import { test, expect, gotoApp, chooseOption, chooseCurrency, selectField, TAB } from './fixtures';

const ledgerRow = (page: import('@playwright/test').Page, text: string) =>
  page.getByTestId('ledger-row').filter({ hasText: text });

test('records an expense', async ({ app, api }) => {
  await gotoApp(app, TAB.finance);

  await app.getByPlaceholder('15.50').fill('15.50');
  await chooseCurrency(app, 'EUR');
  await app.getByPlaceholder('What was it for?').fill('Team lunch');
  await app.getByRole('button', { name: 'Record' }).click();

  await expect(ledgerRow(app, 'Team lunch')).toBeVisible();

  const tx = api.transactions.find((t) => t.note === 'Team lunch')!;
  expect(tx.direction).toBe('expense');
  expect(tx.amount_minor).toBe(1550);
  expect(tx.currency).toBe('EUR');
});

test('records income', async ({ app, api }) => {
  await gotoApp(app, TAB.finance);

  await chooseOption(selectField(app, 'Direction'), 'Income');
  await app.getByPlaceholder('15.50').fill('2500');
  await chooseCurrency(app, 'EUR');
  await app.getByPlaceholder('What was it for?').fill('Salary');
  await app.getByRole('button', { name: 'Record' }).click();

  await expect(ledgerRow(app, 'Salary')).toBeVisible();
  expect(api.transactions.find((t) => t.note === 'Salary')?.direction).toBe('income');
});

/// The value that motivates the whole minor-unit design.
test('an amount with cents is stored exactly', async ({ app, api }) => {
  await gotoApp(app, TAB.finance);

  await app.getByPlaceholder('15.50').fill('0.29');
  await chooseCurrency(app, 'EUR');
  await app.getByPlaceholder('What was it for?').fill('Exact cents');
  await app.getByRole('button', { name: 'Record' }).click();

  await expect(ledgerRow(app, 'Exact cents')).toBeVisible();
  expect(api.transactions.find((t) => t.note === 'Exact cents')?.amount_minor).toBe(29);
  // And renders back without drift.
  await expect(ledgerRow(app, 'Exact cents')).toContainText('0.29');
});

test('a zero-decimal currency renders without cents', async ({ app, api }) => {
  // JPY is not in the picker, but the API accepts it — a Matrix command can
  // write one. It must still render correctly.
  api.seedTransaction({
    note: 'Ramen',
    currency: 'JPY',
    currency_exponent: 0,
    amount_minor: 500,
  });
  await gotoApp(app, TAB.finance);

  const row = ledgerRow(app, 'Ramen');
  await expect(row).toBeVisible();
  await expect(row).toContainText('500 JPY');
});

test('offers exactly the supported currencies', async ({ app }) => {
  await gotoApp(app, TAB.finance);

  const trigger = selectField(app, 'Currency');
  await trigger.click();

  // The container holds the trigger plus the popover options, so drop the
  // trigger to assert on the offered list alone. `CURRENCY_OPTIONS` (shared
  // with the preferences modal) carries a descriptive label per code, so this
  // checks the leading code rather than the full text.
  const options = trigger.locator('..').getByRole('button');
  const codes = (await options.allInnerTexts()).slice(1).map((text) => text.slice(0, 3));
  expect(codes).toEqual([
    'CNY', 'USD', 'EUR', 'GBP', 'JPY', 'HKD', 'SGD', 'AUD', 'CAD', 'CHF', 'INR',
  ]);
});

test('surfaces a server rejection for an over-precise amount', async ({ app, api }) => {
  await gotoApp(app, TAB.finance);

  await app.getByPlaceholder('15.50').fill('15.501');
  await chooseCurrency(app, 'EUR');
  await app.getByRole('button', { name: 'Record' }).click();

  await expect(app.getByText(/decimal place/)).toBeVisible();
  expect(api.transactions).toHaveLength(0);
});

test('will not submit without an amount', async ({ app, api }) => {
  await gotoApp(app, TAB.finance);

  await app.getByRole('button', { name: 'Record' }).click();

  await expect(app.getByText('Enter an amount.')).toBeVisible();
  expect(api.transactions).toHaveLength(0);
});

test('filters the ledger by direction', async ({ app, api }) => {
  api.seedTransaction({ note: 'Groceries', direction: 'expense' });
  api.seedTransaction({ note: 'Paycheck', direction: 'income', amount_minor: 250000 });
  await gotoApp(app, TAB.finance);

  await expect(ledgerRow(app, 'Groceries')).toBeVisible();
  await expect(ledgerRow(app, 'Paycheck')).toBeVisible();

  await app.getByRole('button', { name: 'expense', exact: true }).click();
  await expect(ledgerRow(app, 'Groceries')).toBeVisible();
  await expect(ledgerRow(app, 'Paycheck')).toHaveCount(0);

  await app.getByRole('button', { name: 'income', exact: true }).click();
  await expect(ledgerRow(app, 'Groceries')).toHaveCount(0);
  await expect(ledgerRow(app, 'Paycheck')).toBeVisible();
});

test('deletes an entry', async ({ app, api }) => {
  api.seedTransaction({ note: 'Mistake' });
  await gotoApp(app, TAB.finance);

  await ledgerRow(app, 'Mistake').getByTitle('Delete entry').click();

  await expect(ledgerRow(app, 'Mistake')).toHaveCount(0);
  expect(api.transactions).toHaveLength(0);
});

test('shows an empty state', async ({ app }) => {
  await gotoApp(app, TAB.finance);
  await expect(app.getByText('Nothing recorded yet.')).toBeVisible();
});

test.describe('per-currency summary', () => {
  test('never mixes currencies into one number', async ({ app, api }) => {
    api.seedTransaction({ currency: 'EUR', currency_exponent: 2, amount_minor: 2000 });
    api.seedTransaction({
      currency: 'JPY',
      currency_exponent: 0,
      amount_minor: 300000,
      direction: 'income',
    });
    await gotoApp(app, TAB.finance);

    const summary = app.getByTestId('finance-summary');
    await expect(summary.getByText('EUR THIS MONTH')).toBeVisible();
    await expect(summary.getByText('JPY THIS MONTH')).toBeVisible();
    // EUR is 20.00 spent, so the net is negative; JPY is income.
    await expect(summary).toContainText('-20.00');
    await expect(summary).toContainText('300000');
  });
});

test.describe('categories', () => {
  test('creates a category and assigns it to an entry', async ({ app, api }) => {
    await gotoApp(app, TAB.finance);

    await app.getByRole('button', { name: 'Categories' }).click();
    await app.getByPlaceholder('New category name').fill('food');
    await app.getByRole('button', { name: 'Add', exact: true }).click();

    await expect(app.getByTestId('category-manager').getByText('food')).toBeVisible();

    await app.getByPlaceholder('15.50').fill('12.00');
    await chooseCurrency(app, 'EUR');
    await app.getByPlaceholder('What was it for?').fill('Dinner');
    await chooseOption(selectField(app, 'Category'), 'food');
    await app.getByRole('button', { name: 'Record' }).click();

    await expect(ledgerRow(app, 'Dinner')).toContainText('food');
    expect(api.transactions.find((t) => t.note === 'Dinner')?.category_name).toBe('food');
  });

  test('rejects a duplicate category name', async ({ app, api }) => {
    api.seedCategory('food');
    await gotoApp(app, TAB.finance);

    await app.getByRole('button', { name: 'Categories' }).click();
    await app.getByPlaceholder('New category name').fill('food');
    await app.getByRole('button', { name: 'Add', exact: true }).click();

    await expect(app.getByText(/already have a category/)).toBeVisible();
    expect(api.categories).toHaveLength(1);
  });

  /// Money is never destroyed by tidying the category list.
  test('deleting a category keeps its entries, uncategorised', async ({ app, api }) => {
    const category = api.seedCategory('food');
    api.seedTransaction({
      note: 'Lunch',
      category_id: category.id,
      category_name: 'food',
    });
    await gotoApp(app, TAB.finance);

    await expect(ledgerRow(app, 'Lunch')).toContainText('food');

    await app.getByRole('button', { name: 'Categories' }).click();
    await app.getByTitle('Delete category food').click();

    await expect(ledgerRow(app, 'Lunch')).toBeVisible();
    await expect(ledgerRow(app, 'Lunch')).toContainText('Uncategorised');
    expect(api.transactions).toHaveLength(1);
  });

  test('an entry can stay uncategorised', async ({ app, api }) => {
    await gotoApp(app, TAB.finance);

    await app.getByPlaceholder('15.50').fill('4.00');
    await chooseCurrency(app, 'EUR');
    await app.getByPlaceholder('What was it for?').fill('Coffee');
    await app.getByRole('button', { name: 'Record' }).click();

    await expect(ledgerRow(app, 'Coffee')).toContainText('Uncategorised');
    expect(api.transactions.find((t) => t.note === 'Coffee')?.category_id).toBeNull();
  });
});

test.describe('editing a ledger entry', () => {
  const editForm = (page: import('@playwright/test').Page) => page.getByTestId('ledger-row-edit');

  test('changes the amount, note and direction', async ({ app, api }) => {
    const tx = api.seedTransaction({ note: 'Wrong amount', amount_minor: 1000 });
    await gotoApp(app, TAB.finance);

    await ledgerRow(app, 'Wrong amount').getByTitle('Edit entry').click();

    const form = editForm(app);
    // The form is seeded with the stored value, rendered from minor units.
    await expect(form.locator('input[inputmode="decimal"]')).toHaveValue('10.00');

    await form.locator('input[inputmode="decimal"]').fill('12.34');
    await form.getByPlaceholder('What was it for?').fill('Corrected');
    await chooseOption(selectField(form, 'Direction'), 'Income');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(editForm(app)).toHaveCount(0);
    await expect(ledgerRow(app, 'Corrected')).toBeVisible();

    const saved = api.transactions.find((t) => t.id === tx.id)!;
    expect(saved.amount_minor).toBe(1234);
    expect(saved.direction).toBe('income');
    expect(saved.note).toBe('Corrected');
  });

  test('re-categorises an entry', async ({ app, api }) => {
    const category = api.seedCategory('travel');
    const tx = api.seedTransaction({ note: 'Train ticket' });
    await gotoApp(app, TAB.finance);

    await ledgerRow(app, 'Train ticket').getByTitle('Edit entry').click();
    const form = editForm(app);
    await chooseOption(selectField(form, 'Category'), 'travel');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(ledgerRow(app, 'Train ticket')).toContainText('travel');
    expect(api.transactions.find((t) => t.id === tx.id)?.category_id).toBe(category.id);
  });

  test('cancelling discards the change', async ({ app, api }) => {
    api.seedTransaction({ note: 'Leave me alone', amount_minor: 500 });
    await gotoApp(app, TAB.finance);

    await ledgerRow(app, 'Leave me alone').getByTitle('Edit entry').click();
    const form = editForm(app);
    await form.locator('input[inputmode="decimal"]').fill('999.99');
    await form.getByRole('button', { name: 'Cancel' }).click();

    await expect(editForm(app)).toHaveCount(0);
    expect(api.transactions[0].amount_minor).toBe(500);
    expect(api.requests.some((r) => r.method === 'PUT')).toBe(false);
  });

  test('shows a server rejection inside the row', async ({ app, api }) => {
    api.seedTransaction({ note: 'Will fail' });
    await gotoApp(app, TAB.finance);

    await ledgerRow(app, 'Will fail').getByTitle('Edit entry').click();
    const form = editForm(app);
    await form.locator('input[inputmode="decimal"]').fill('1.999');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(form.getByText(/decimal place/)).toBeVisible();
    await expect(editForm(app)).toHaveCount(1);
  });
});

/// A currency the picker does not offer must survive an edit untouched. Without
/// the fallback option the select falls back to its placeholder and saving
/// would blank a field the user never touched.
test('editing an entry preserves a currency outside the picker', async ({ app, api }) => {
  const tx = api.seedTransaction({
    note: 'Ramen',
    currency: 'JPY',
    currency_exponent: 0,
    amount_minor: 500,
  });
  await gotoApp(app, TAB.finance);

  await ledgerRow(app, 'Ramen').getByTitle('Edit entry').click();
  const form = app.getByTestId('ledger-row-edit');

  // The stored currency is shown, not a placeholder.
  await expect(selectField(form, 'Currency')).toHaveText(/JPY/);

  await form.getByPlaceholder('What was it for?').fill('Ramen deluxe');
  await form.getByRole('button', { name: 'Save' }).click();

  await expect(app.getByTestId('ledger-row-edit')).toHaveCount(0);
  const saved = api.transactions.find((t) => t.id === tx.id)!;
  expect(saved.currency).toBe('JPY');
  expect(saved.amount_minor).toBe(500);
});
