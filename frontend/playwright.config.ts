import { defineConfig, devices } from '@playwright/test';

/**
 * Drives the real SPA in a real browser, with the REST API intercepted at the
 * network layer (see `e2e/fake-api.ts`).
 *
 * The backend is not started for these runs. It needs Matrix credentials and a
 * reachable homeserver to boot, which would make the suite non-deterministic
 * and unrunnable in CI; the backend's own behaviour is covered by the Rust
 * tests. What these specs cover is the part those cannot: what the browser
 * actually renders and what a user can actually do.
 */
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'list' : [['list'], ['html', { open: 'never' }]],

  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'on-first-retry',
    // The app stores timestamps in UTC and renders them in the viewer's zone.
    // Pinning both keeps date assertions stable wherever the suite runs.
    timezoneId: 'UTC',
    locale: 'en-US',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 5173',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: !process.env.CI,
    stdout: 'ignore',
    stderr: 'pipe',
  },
});
