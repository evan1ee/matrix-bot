import { test, expect, gotoApp, chooseCurrency, TAB } from './fixtures';
import fs from 'fs';

const holdingRow = (page: import('@playwright/test').Page, text: string) =>
  page.getByTestId('holding-row').filter({ hasText: text });

const gotoPortfolio = (page: import('@playwright/test').Page) => gotoApp(page, TAB.portfolio);

const clickBuy = (page: import('@playwright/test').Page) =>
  page.getByTestId('buy-panel').getByRole('button', { name: 'Buy' }).click();

/** Comfortably more cash than any single test spends. */
const AMPLE_CASH = 100_000_00;

test('buys a new position', async ({ app, api }) => {
  api.seedCash('USD', AMPLE_CASH);
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('aapl');
  await app.getByPlaceholder('10').fill('10');
  await app.getByPlaceholder('150.00').fill('150.00');
  await app.getByPlaceholder('Note (optional)').fill('core position');
  await clickBuy(app);

  const row = holdingRow(app, 'AAPL');
  await expect(row).toBeVisible();
  await expect(row).toContainText('10');
  await expect(row).toContainText('150.00');
  await expect(row).toContainText('core position');

  const holding = api.holdings.find((h) => h.symbol === 'AAPL')!;
  expect(holding.quantity_minor).toBe(10 * 10 ** 8);
  expect(holding.avg_cost_minor).toBe(15000);
  expect(holding.currency).toBe('USD');
});

/// The value that motivates the minor-unit design applies to quantity too.
test('a fractional quantity is stored exactly', async ({ app, api }) => {
  api.seedCash('USD', AMPLE_CASH);
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('BTC');
  await app.getByPlaceholder('10').fill('0.5');
  await app.getByPlaceholder('150.00').fill('60000');
  await clickBuy(app);

  await expect(holdingRow(app, 'BTC')).toContainText('0.5');
  expect(api.holdings.find((h) => h.symbol === 'BTC')?.quantity_minor).toBe(0.5 * 10 ** 8);
});

test('buying more of the same symbol blends the average cost', async ({ app, api }) => {
  api.seedHolding({ symbol: 'AAPL', quantity_minor: 10 * 10 ** 8, avg_cost_minor: 15000 });
  api.seedCash('USD', AMPLE_CASH);
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('AAPL');
  await app.getByPlaceholder('10').fill('5');
  await app.getByPlaceholder('150.00').fill('180.00');
  await clickBuy(app);

  const holding = api.holdings.find((h) => h.symbol === 'AAPL')!;
  expect(holding.quantity_minor).toBe(15 * 10 ** 8);
  // (10x150 + 5x180) / 15 = 160.00
  expect(holding.avg_cost_minor).toBe(16000);
  await expect(holdingRow(app, 'AAPL')).toContainText('160.00');
});

test('a currency mismatch for the same symbol is rejected by the server', async ({ app, api }) => {
  api.seedHolding({ symbol: 'AAPL', currency: 'USD', currency_exponent: 2 });
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('AAPL');
  await app.getByPlaceholder('10').fill('5');
  await app.getByPlaceholder('150.00').fill('180.00');
  await chooseCurrency(app.getByTestId('buy-panel'), 'EUR');
  await clickBuy(app);

  await expect(app.getByText(/already held in USD/)).toBeVisible();
  expect(api.holdings).toHaveLength(1);
});

/// The whole point of tracking cash: a buy with nothing deposited must be
/// rejected, not silently succeed.
test('a buy without enough cash is rejected and creates no position', async ({ app, api }) => {
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('AAPL');
  await app.getByPlaceholder('10').fill('10');
  await app.getByPlaceholder('150.00').fill('150.00');
  await clickBuy(app);

  await expect(app.getByText(/Need/)).toBeVisible();
  expect(api.holdings).toHaveLength(0);
});

test('surfaces a server rejection for an over-precise price', async ({ app, api }) => {
  await gotoPortfolio(app);

  await app.getByPlaceholder('AAPL').fill('AAPL');
  await app.getByPlaceholder('10').fill('10');
  await app.getByPlaceholder('150.00').fill('15.501');
  await clickBuy(app);

  await expect(app.getByText(/decimal place/)).toBeVisible();
  expect(api.holdings).toHaveLength(0);
});

test('will not submit without a symbol, quantity, and price', async ({ app, api }) => {
  await gotoPortfolio(app);

  await clickBuy(app);

  await expect(app.getByText('Enter a symbol, quantity, and price.')).toBeVisible();
  expect(api.holdings).toHaveLength(0);
});

test('shows an empty state', async ({ app }) => {
  await gotoPortfolio(app);
  await expect(app.getByText('No holdings yet.')).toBeVisible();
});

test.describe('setting a price', () => {
  test('records the current price and shows market value and unrealized P&L', async ({ app, api }) => {
    api.seedHolding({ symbol: 'AAPL', quantity_minor: 10 * 10 ** 8, avg_cost_minor: 15000 });
    await gotoPortfolio(app);

    const row = holdingRow(app, 'AAPL');
    await row.getByText('set current price').click();
    await row.locator('input[inputmode="decimal"]').fill('180.00');
    await row.getByTitle('Save price').click();

    // Market value: 10 * 180.00 = 1800.00. Cost basis: 10 * 150.00 = 1500.00.
    // Unrealized gain: +300.00.
    await expect(row).toContainText('1800.00');
    await expect(row).toContainText('+300.00');

    const holding = api.holdings.find((h) => h.symbol === 'AAPL')!;
    expect(holding.current_price_minor).toBe(18000);
  });

  test('a holding at a loss shows a negative unrealized P&L', async ({ app, api }) => {
    api.seedHolding({ symbol: 'AAPL', quantity_minor: 10 * 10 ** 8, avg_cost_minor: 15000 });
    await gotoPortfolio(app);

    const row = holdingRow(app, 'AAPL');
    await row.getByText('set current price').click();
    await row.locator('input[inputmode="decimal"]').fill('120.00');
    await row.getByTitle('Save price').click();

    await expect(row).toContainText('-300.00');
  });

  test('an unpriced holding says so rather than showing a value', async ({ app, api }) => {
    api.seedHolding({ symbol: 'AAPL' });
    await gotoPortfolio(app);

    await expect(holdingRow(app, 'AAPL')).toContainText('no price set');
  });
});

test.describe('selling', () => {
  test('a partial sell reduces the position and reports realized P&L', async ({ app, api }) => {
    const holding = api.seedHolding({
      symbol: 'AAPL',
      quantity_minor: 10 * 10 ** 8,
      avg_cost_minor: 15000,
    });
    await gotoPortfolio(app);

    await holdingRow(app, 'AAPL').getByTitle('Sell').click();
    const form = app.getByTestId('sell-row');
    await form.getByPlaceholder('e.g. 5').fill('4');
    await form.getByPlaceholder('e.g. 180.00').fill('180.00');
    await form.getByRole('button', { name: 'Sell' }).click();

    await expect(app.getByTestId('sell-row')).toHaveCount(0);
    await expect(holdingRow(app, 'AAPL')).toContainText('6');

    const updated = api.holdings.find((h) => h.id === holding.id)!;
    expect(updated.quantity_minor).toBe(6 * 10 ** 8);
    // Realized: 4 shares x ($180 - $150) = $120.00 gain.
    expect(
      api.requests.some((r) => r.method === 'POST' && r.url === `/portfolio/holdings/${holding.id}/sell`)
    ).toBe(true);
  });

  test('selling the full quantity closes the position', async ({ app, api }) => {
    const holding = api.seedHolding({
      symbol: 'AAPL',
      quantity_minor: 10 * 10 ** 8,
      avg_cost_minor: 15000,
    });
    await gotoPortfolio(app);

    await holdingRow(app, 'AAPL').getByTitle('Sell').click();
    const form = app.getByTestId('sell-row');
    await form.getByPlaceholder('e.g. 5').fill('10');
    await form.getByPlaceholder('e.g. 180.00').fill('180.00');
    await form.getByRole('button', { name: 'Sell' }).click();

    await expect(holdingRow(app, 'AAPL')).toHaveCount(0);
    expect(api.holdings.find((h) => h.id === holding.id)).toBeUndefined();
  });

  test('rejects selling more than is held', async ({ app, api }) => {
    api.seedHolding({ symbol: 'AAPL', quantity_minor: 10 * 10 ** 8, avg_cost_minor: 15000 });
    await gotoPortfolio(app);

    await holdingRow(app, 'AAPL').getByTitle('Sell').click();
    const form = app.getByTestId('sell-row');
    await form.getByPlaceholder('e.g. 5').fill('11');
    await form.getByPlaceholder('e.g. 180.00').fill('180.00');
    await form.getByRole('button', { name: 'Sell' }).click();

    await expect(form.getByText(/Only \d+ held/)).toBeVisible();
    expect(api.holdings[0].quantity_minor).toBe(10 * 10 ** 8);
  });

  test('cancelling discards the sell', async ({ app, api }) => {
    api.seedHolding({ symbol: 'AAPL', quantity_minor: 10 * 10 ** 8, avg_cost_minor: 15000 });
    await gotoPortfolio(app);

    await holdingRow(app, 'AAPL').getByTitle('Sell').click();
    const form = app.getByTestId('sell-row');
    await form.getByPlaceholder('e.g. 5').fill('4');
    await form.getByRole('button', { name: 'Cancel' }).click();

    await expect(app.getByTestId('sell-row')).toHaveCount(0);
    expect(api.holdings[0].quantity_minor).toBe(10 * 10 ** 8);
    expect(api.requests.some((r) => r.method === 'POST' && r.url.endsWith('/sell'))).toBe(false);
  });
});

test('removes a holding entirely', async ({ app, api }) => {
  api.seedHolding({ symbol: 'AAPL' });
  await gotoPortfolio(app);

  await holdingRow(app, 'AAPL').getByTitle('Remove holding').click();
  await app.getByRole('button', { name: 'Remove Position' }).click();

  await expect(holdingRow(app, 'AAPL')).toHaveCount(0);
  expect(api.holdings).toHaveLength(0);
});

test.describe('cash', () => {
  test('deposits and shows the balance', async ({ app, api }) => {
    await gotoPortfolio(app);

    const panel = app.getByTestId('cash-panel');
    await panel.getByPlaceholder('500.00').fill('1000');
    await panel.getByRole('button', { name: 'Deposit' }).click();

    await expect(app.getByTestId('cash-balances')).toContainText('1000.00 USD');
    expect(api.cash.find((c) => c.currency === 'USD')?.balance_minor).toBe(100000);
  });

  test('withdraws and reduces the balance', async ({ app, api }) => {
    api.seedCash('USD', 100000);
    await gotoPortfolio(app);

    const panel = app.getByTestId('cash-panel');
    await panel.getByPlaceholder('500.00').fill('400');
    await panel.getByRole('button', { name: 'Withdraw' }).click();

    await expect(app.getByTestId('cash-balances')).toContainText('600.00 USD');
  });

  test('rejects a withdrawal larger than the balance', async ({ app, api }) => {
    api.seedCash('USD', 10000);
    await gotoPortfolio(app);

    const panel = app.getByTestId('cash-panel');
    await panel.getByPlaceholder('500.00').fill('100.01');
    await panel.getByRole('button', { name: 'Withdraw' }).click();

    await expect(panel.getByText(/available/)).toBeVisible();
    expect(api.cash.find((c) => c.currency === 'USD')?.balance_minor).toBe(10000);
  });

  test('a buy debits cash and a sell credits it back', async ({ app, api }) => {
    api.seedCash('USD', AMPLE_CASH);
    await gotoPortfolio(app);

    await app.getByPlaceholder('AAPL').fill('AAPL');
    await app.getByPlaceholder('10').fill('10');
    await app.getByPlaceholder('150.00').fill('150.00');
    await clickBuy(app);

    // 100000.00 - 1500.00 (10 @ 150.00) = 98500.00.
    await expect(app.getByTestId('cash-balances')).toContainText('98500.00 USD');

    await holdingRow(app, 'AAPL').getByTitle('Sell').click();
    const form = app.getByTestId('sell-row');
    await form.getByPlaceholder('e.g. 5').fill('10');
    await form.getByPlaceholder('e.g. 180.00').fill('180.00');
    await form.getByRole('button', { name: 'Sell' }).click();

    // 98500.00 + 1800.00 (10 @ 180.00 proceeds) = 100300.00.
    await expect(app.getByTestId('cash-balances')).toContainText('100300.00 USD');
  });
});

test.describe('per-currency summary', () => {
  test('never mixes currencies into one number', async ({ app, api }) => {
    api.seedHolding({
      symbol: 'AAPL',
      currency: 'USD',
      currency_exponent: 2,
      quantity_minor: 10 * 10 ** 8,
      avg_cost_minor: 15000,
      current_price_minor: 18000,
    });
    api.seedHolding({
      symbol: 'SONY',
      currency: 'JPY',
      currency_exponent: 0,
      quantity_minor: 100 * 10 ** 8,
      avg_cost_minor: 2000,
    });
    await gotoPortfolio(app);

    const summary = app.getByTestId('portfolio-summary');
    await expect(summary.getByText('USD PORTFOLIO')).toBeVisible();
    await expect(summary.getByText('JPY PORTFOLIO')).toBeVisible();
    // USD: 10 shares gained $30 each = +300.00.
    await expect(summary).toContainText('+300.00');
    // JPY has no price set, so it must call out the unpriced holding rather
    // than silently reporting a zero P&L.
    await expect(summary).toContainText('1 unpriced');
  });

  /// A deposit with nothing bought yet must still be visible — it is real
  /// money on record, not nothing.
  test('a currency with only a cash deposit still appears', async ({ app, api }) => {
    api.seedCash('GBP', 50000);
    await gotoPortfolio(app);

    const summary = app.getByTestId('portfolio-summary');
    await expect(summary.getByText('GBP PORTFOLIO')).toBeVisible();
    await expect(summary).toContainText('cash 500.00');
  });

  test('renders net worth performance chart with timeframe toggles and captures screenshots', async ({
    app,
    api,
  }) => {
    api.seedCash('USD', AMPLE_CASH);
    api.seedHolding({
      symbol: 'BTC-USD',
      currency: 'USD',
      currency_exponent: 2,
      quantity_minor: 1 * 10 ** 8,
      avg_cost_minor: 6000000,
      current_price_minor: 6850000,
    });
    api.seedHolding({
      symbol: 'NVDA',
      currency: 'USD',
      currency_exponent: 2,
      quantity_minor: 25 * 10 ** 8,
      avg_cost_minor: 11000,
      current_price_minor: 12850,
    });
    await gotoPortfolio(app);

    // Verify Performance Chart
    await expect(app.getByText('NET WORTH HISTORY (USD)')).toBeVisible();
    await expect(app.getByRole('button', { name: '7D' })).toBeVisible();
    await expect(app.getByRole('button', { name: '1M' })).toBeVisible();
    await expect(app.getByRole('button', { name: '1Y' })).toBeVisible();

    const chartCard = app.getByText('NET WORTH HISTORY (USD)').locator('xpath=ancestor::div[contains(@class, "rounded-xl")]').first();
    const chartSvg = chartCard.locator('svg.select-none').first();
    const header = chartCard.locator('> div').first();

    for (const vpWidth of [375, 500, 640, 768, 1024, 1280]) {
      await app.setViewportSize({ width: vpWidth, height: 800 });
      await app.waitForTimeout(200);

      // Move mouse away to unhover
      await app.mouse.move(0, 0);
      await app.waitForTimeout(100);

      const boxBefore = await chartCard.boundingBox();
      const svgBefore = await chartSvg.boundingBox();
      const headerBefore = await header.boundingBox();

      // Hover over chart
      await chartSvg.scrollIntoViewIfNeeded();
      await chartSvg.hover({ position: { x: 100, y: 50 } });
      await app.waitForTimeout(200);

      const boxHover = await chartCard.boundingBox();
      const svgHover = await chartSvg.boundingBox();
      const headerHover = await header.boundingBox();

      const diff = (boxHover?.height ?? 0) - (boxBefore?.height ?? 0);
      const svgDiff = (svgHover?.height ?? 0) - (svgBefore?.height ?? 0);
      const headerDiff = (headerHover?.height ?? 0) - (headerBefore?.height ?? 0);
      expect(Math.abs(diff)).toBeLessThanOrEqual(1);
      expect(Math.abs(headerDiff)).toBeLessThanOrEqual(1);
      expect(Math.abs(svgDiff)).toBeLessThanOrEqual(1);
    }

    // Capture focused Performance Chart screenshot in unhovered and hovered states
    const artifactDir = process.env.BRAIN_DIR;
    if (artifactDir && fs.existsSync(artifactDir)) {
      await app.setViewportSize({ width: 1280, height: 800 });
      await app.mouse.move(0, 0);
      await app.waitForTimeout(100);
      await chartCard.scrollIntoViewIfNeeded();
      await chartCard.screenshot({
        path: `${artifactDir}/portfolio_chart_unhovered.png`,
      });

      await chartSvg.hover({ position: { x: 100, y: 50 } });
      await app.waitForTimeout(200);
      await chartCard.screenshot({
        path: `${artifactDir}/portfolio_chart_hovered.png`,
      });

      // Capture main dashboard view scrolled to the chart & holdings
      await app.screenshot({
        path: `${artifactDir}/portfolio_performance_dashboard.png`,
      });
    }

    // Capture holdings list with the Price Alerts toolbar button and holding row bell triggers
    const holdingsPanel = app.getByText('HOLDINGS').locator('xpath=ancestor::div[contains(@class, "rounded-lg")]').first();
    await holdingsPanel.scrollIntoViewIfNeeded();
    if (artifactDir && fs.existsSync(artifactDir)) {
      await holdingsPanel.screenshot({
        path: `${artifactDir}/portfolio_holdings_with_alerts_trigger.png`,
      });
    }

    // Open Alerts Modal via toolbar button
    await app.getByRole('button', { name: 'Price Alerts' }).click();
    await expect(app.getByText('Market Price & Volatility Alerts')).toBeVisible();
    await expect(app.getByText('Create New Price Alert')).toBeVisible();

    // Fill and create an alert
    await app.getByPlaceholder('BTC-USD, AAPL').fill('BTC-USD');
    await app.getByPlaceholder('e.g. 70000').fill('70000');
    await app.getByRole('button', { name: 'Add', exact: true }).click();

    // Verify alert appears in list
    await expect(app.getByText('BTC-USD')).toBeVisible();
    await expect(app.getByText('Active', { exact: true })).toBeVisible();

    // Capture alerts modal screenshot
    if (artifactDir && fs.existsSync(artifactDir)) {
      await app.screenshot({
        path: `${artifactDir}/portfolio_alerts_modal.png`,
      });
    }
  });
});
