import { test, expect, gotoApp, TAB } from './fixtures';

const gotoFootprints = (page: import('@playwright/test').Page) => gotoApp(page, TAB.footprints);

test.describe('Footprints (Travel & Places Visited)', () => {
  test('renders empty footprints view with map, stats, and add prompt', async ({ app }) => {
    await gotoFootprints(app);

    await expect(app.getByRole('heading', { name: /Footprints & Travel Journal/i })).toBeVisible();
    await expect(app.getByText('Places Visited')).toBeVisible();
    await expect(app.getByText('Countries')).toBeVisible();
    await expect(app.getByText('Days Traveled')).toBeVisible();
    await expect(app.getByText('Recorded Spend')).toBeVisible();

    await expect(app.getByText('No footprints found')).toBeVisible();
    await expect(app.getByRole('button', { name: /Record First Place/i })).toBeVisible();
  });

  test('records a new visited place with duration, rating, spend, and notes', async ({ app, api }) => {
    await gotoFootprints(app);

    await app.getByRole('button', { name: /Add Place/i }).first().click();
    await expect(app.getByRole('heading', { name: 'Record Visited Place' })).toBeVisible();

    // Fill form fields
    await app.getByPlaceholder('e.g. Grand Canyon').fill('Grand Canyon');
    await app.getByPlaceholder('e.g. Paris').fill('Arizona');
    await app.getByPlaceholder('e.g. France').fill('United States');
    await app.getByPlaceholder('0.00').fill('150.00');
    await app.getByPlaceholder(/What made this place special/i).fill('Unforgettable canyon hiking memories!');

    await app.getByRole('button', { name: 'Save Footprint' }).click();

    // Card heading should appear in list
    await expect(app.getByRole('heading', { name: 'Grand Canyon' })).toBeVisible();
    await expect(app.getByText('Arizona, United States')).toBeVisible();
    await expect(app.getByText('150.00 USD').first()).toBeVisible();
    await expect(app.getByText('"Unforgettable canyon hiking memories!"')).toBeVisible();

    // Map star marker should also be visible on map
    await expect(app.locator('.footprint-star-div-icon')).toBeVisible();

    // Verification against fake api
    const created = api.footprints.find((f) => f.title === 'Grand Canyon');
    expect(created).toBeDefined();
    expect(created?.spend_amount_minor).toBe(15000);
    expect(created?.rating).toBe(5);
  });

  test('filters footprints by category and search query', async ({ app, api }) => {
    api.seedFootprint({
      title: 'Eiffel Tower',
      country: 'France',
      city: 'Paris',
      category: 'sightseeing',
      rating: 5,
    });
    api.seedFootprint({
      title: 'Ichiran Ramen',
      country: 'Japan',
      city: 'Tokyo',
      category: 'food',
      rating: 5,
    });

    await gotoFootprints(app);

    await expect(app.getByRole('heading', { name: 'Eiffel Tower' })).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Ichiran Ramen' })).toBeVisible();

    // Filter by 'Food'
    await app.getByRole('button', { name: 'Food', exact: true }).click();
    await expect(app.getByRole('heading', { name: 'Ichiran Ramen' })).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Eiffel Tower' })).not.toBeVisible();

    // Switch back to 'All' and search
    await app.getByRole('button', { name: 'All', exact: true }).click();
    await app.getByPlaceholder(/Search places, cities/i).fill('Paris');
    await expect(app.getByRole('heading', { name: 'Eiffel Tower' })).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Ichiran Ramen' })).not.toBeVisible();
  });

  test('deletes a footprint entry', async ({ app, api }) => {
    api.seedFootprint({
      title: 'Colosseum',
      country: 'Italy',
      city: 'Rome',
      category: 'sightseeing',
      rating: 5,
    });

    await gotoFootprints(app);
    await expect(app.getByRole('heading', { name: 'Colosseum' })).toBeVisible();

    // Delete
    await app.getByTitle('Delete Footprint').click();
    await expect(app.getByRole('heading', { name: 'Colosseum' })).not.toBeVisible();
    expect(api.footprints.length).toBe(0);
  });
});
