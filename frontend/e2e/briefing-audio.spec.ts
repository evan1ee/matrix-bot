import { test, expect, gotoApp, TAB } from './fixtures';

test.describe('Morning briefing audio', () => {
  test('shows no player when today has no recording', async ({ app }) => {
    await gotoApp(app, TAB.dashboard);
    await expect(app.getByText('Welcome back')).toBeVisible();
    await expect(app.getByTestId('briefing-card')).toHaveCount(0);
  });

  test('plays today\'s briefing with speed controls and a download link', async ({ app, api }) => {
    api.briefingAudio = true;

    await gotoApp(app, TAB.dashboard);
    const card = app.getByTestId('briefing-card');
    await expect(card).toBeVisible();
    await expect(app.getByTestId('briefing-audio')).toHaveAttribute('src', '/fake-media/briefing.ogg');
    await expect(card.getByRole('link', { name: 'Download' })).toHaveAttribute(
      'href',
      '/fake-media/briefing.ogg'
    );

    await card.getByRole('button', { name: '1.5x' }).click();
    await expect(card.getByRole('button', { name: '1.5x' })).toHaveAttribute('aria-pressed', 'true');
    expect(await app.getByTestId('briefing-audio').evaluate((el: HTMLAudioElement) => el.playbackRate)).toBe(1.5);
  });
});
