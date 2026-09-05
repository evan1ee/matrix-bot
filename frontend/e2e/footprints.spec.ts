import { test, expect, gotoApp, TAB } from './fixtures';
import path from 'path';
import fs from 'fs';

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

  test('captures screenshots of top-and-down map and grid card layout in dark and light modes', async ({
    app,
    api,
  }) => {
    // Seed real-world places similar to user data
    api.seedFootprint({
      title: 'Danxia Mountain (丹霞山)',
      country: 'China',
      city: 'Shaoguan',
      category: 'sightseeing',
      rating: 5,
      spend_amount_minor: 100000,
      spend_currency: 'USD',
      note: 'Red sandstone cliffs, natural UNESCO world heritage site.',
      stay_duration_days: 10,
      arrival_date: '2026-09-04',
      departure_date: '2026-09-14',
      latitude: 25.04,
      longitude: 113.74,
    });
    api.seedFootprint({
      title: 'Xiaojing Bay (小径湾)',
      country: 'China',
      city: 'Huizhou',
      category: 'lodging',
      rating: 4,
      spend_amount_minor: 42000,
      spend_currency: 'USD',
      note: 'Relaxing coastal stay, ocean waves and sunset strolls.',
      stay_duration_days: 3,
      arrival_date: '2026-09-01',
      departure_date: '2026-09-04',
      latitude: 22.79,
      longitude: 114.73,
    });
    api.seedFootprint({
      title: 'Huizhou West Lake (惠州西湖)',
      country: 'China',
      city: 'Huizhou',
      category: 'sightseeing',
      rating: 5,
      spend_amount_minor: 8500,
      spend_currency: 'USD',
      note: 'Pagodas across misty water reflections.',
      stay_duration_days: 1,
      arrival_date: '2026-08-30',
      latitude: 23.09,
      longitude: 114.39,
    });
    api.seedFootprint({
      title: 'Grand Canyon',
      country: 'United States',
      city: 'Arizona',
      category: 'nature',
      rating: 5,
      spend_amount_minor: 15000,
      spend_currency: 'USD',
      note: 'Unforgettable canyon hiking memories with panoramic vistas!',
      stay_duration_days: 4,
      arrival_date: '2026-07-12',
      departure_date: '2026-07-16',
      latitude: 36.05,
      longitude: -112.14,
    });
    api.seedFootprint({
      title: 'Ichiran Ramen',
      country: 'Japan',
      city: 'Tokyo',
      category: 'food',
      rating: 5,
      spend_amount_minor: 3500,
      spend_currency: 'USD',
      note: 'Solo dining booth, rich tonkotsu broth.',
      stay_duration_days: 1,
      arrival_date: '2026-06-20',
      latitude: 35.66,
      longitude: 139.70,
    });

    await gotoFootprints(app);

    await expect(app.getByRole('heading', { name: 'Danxia Mountain (丹霞山)' })).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Xiaojing Bay (小径湾)' })).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Huizhou West Lake (惠州西湖)' })).toBeVisible();

    await app.setViewportSize({ width: 1280, height: 1300 });
    // Wait for map tiles to load
    await app.waitForTimeout(2500);

    const brainDir = process.env.BRAIN_DIR;
    if (brainDir && fs.existsSync(brainDir)) {
      // 1. Dark Mode full-page screenshot
      await app.screenshot({
        path: path.join(brainDir, 'footprints_topdown_dark.png'),
        fullPage: true,
      });

      // 2. Switch to Light Mode
      const themeToggle = app.locator('button[title*="light mode" i]:visible, button[title*="dark mode" i]:visible').first();
      if (await themeToggle.isVisible()) {
        await themeToggle.click();
        await app.waitForTimeout(500);

        await app.screenshot({
          path: path.join(brainDir, 'footprints_topdown_light.png'),
          fullPage: true,
        });

        // Switch back to dark mode
        await themeToggle.click();
        await app.waitForTimeout(300);
      }
    }
  });
});
