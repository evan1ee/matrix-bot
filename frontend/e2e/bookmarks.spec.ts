import { test, expect, gotoApp, TAB } from './fixtures';

const gotoBookmarks = (page: import('@playwright/test').Page) => gotoApp(page, TAB.bookmarks);

test.describe('Bookmarks & Read-it-Later Library', () => {
  test('renders empty bookmarks view with masthead, stat strip, and empty state prompt', async ({ app }) => {
    await gotoBookmarks(app);

    await expect(app.getByRole('heading', { name: /Bookmarks & Reading List/i })).toBeVisible();
    await expect(app.getByText(/Clean reader view, metadata extraction, and tag categorization/i)).toBeVisible();

    await expect(app.getByText('Unread', { exact: true })).toBeVisible();
    await expect(app.getByText('Favorites', { exact: true })).toBeVisible();
    await expect(app.getByText('Total Saved', { exact: true })).toBeVisible();
    await expect(app.getByText('Archived', { exact: true })).toBeVisible();

    await expect(app.getByText('No bookmarks found')).toBeVisible();
    await expect(app.getByTestId('save-bookmark-btn')).toBeVisible();
  });

  test('saves a new bookmark and displays it in unread list with domain and tags', async ({ app, api }) => {
    await gotoBookmarks(app);

    await app.getByTestId('save-bookmark-btn').click();
    await expect(app.getByRole('heading', { name: 'Save New Bookmark' })).toBeVisible();

    await app.getByPlaceholder(/https:\/\/example.com\/article/i).fill('https://news.ycombinator.com/item?id=12345');
    await app.getByPlaceholder(/Leave blank to auto-extract/i).fill('Show HN: Matrix Hello Bot');
    await app.getByPlaceholder(/tech, reading, rust/i).fill('matrix, rust, ai');

    await app.getByRole('button', { name: 'Save Bookmark' }).click();

    // Check that modal closes and card is rendered
    const card = app.getByTestId(/^bookmark-card-\d+/).first();
    await expect(card.getByText('Show HN: Matrix Hello Bot')).toBeVisible();
    await expect(card.getByText(/news.ycombinator.com/i)).toBeVisible();
    await expect(card.getByText('#matrix')).toBeVisible();
    await expect(card.getByText('#rust')).toBeVisible();
    await expect(card.getByText('#ai')).toBeVisible();

    const created = api.bookmarks.find((b) => b.title === 'Show HN: Matrix Hello Bot');
    expect(created).toBeDefined();
    expect(created?.domain).toBe('news.ycombinator.com');
  });

  test('saves a bookmark without title and falls back to auto-extracted title', async ({ app, api }) => {
    await gotoBookmarks(app);

    await app.getByTestId('save-bookmark-btn').click();
    await expect(app.getByRole('heading', { name: 'Save New Bookmark' })).toBeVisible();

    await app.getByPlaceholder(/https:\/\/example.com\/article/i).fill('https://www.youtube.com/watch?v=i_1f5OEJIAY');
    await app.getByPlaceholder(/tech, reading, rust/i).fill('tech');

    await app.getByRole('button', { name: 'Save Bookmark' }).click();

    // Dialog closes
    await expect(app.getByRole('heading', { name: 'Save New Bookmark' })).not.toBeVisible();
    const card = app.getByTestId(/^bookmark-card-\d+/).first();
    await expect(card).toBeVisible();
    await expect(card.getByText('#tech')).toBeVisible();
  });

  test('stars a bookmark to toggle favorite status', async ({ app, api }) => {
    const bm = api.seedBookmark({
      url: 'https://doc.rust-lang.org/book/',
      title: 'The Rust Programming Language',
      tags: 'rust,docs',
      is_favorite: false,
    });

    await gotoBookmarks(app);
    await expect(app.getByText('The Rust Programming Language')).toBeVisible();

    // Click star button
    const card = app.getByTestId(`bookmark-card-${bm.id}`);
    await card.getByRole('button', { name: /Star bookmark/i }).click();

    // Verify it updated in fake API
    await expect.poll(() => api.bookmarks.find((b) => b.id === bm.id)?.is_favorite).toBe(true);

    // Switch to Favorites tab
    await app.getByTestId('tab-favorites').click();
    await expect(app.getByText('The Rust Programming Language')).toBeVisible();
  });

  test('archives a bookmark and moves it out of unread tab', async ({ app, api }) => {
    const bm = api.seedBookmark({
      url: 'https://fly.io/blog/all-in-on-sqlite/',
      title: 'All-in on SQLite #litestream',
      tags: 'sqlite,infra',
      is_archived: false,
    });

    await gotoBookmarks(app);
    await expect(app.getByText('All-in on SQLite #litestream')).toBeVisible();

    // Archive it
    const card = app.getByTestId(`bookmark-card-${bm.id}`);
    await card.getByRole('button', { name: /Archive bookmark/i }).click();

    // Verify it moved out of Unread tab
    await expect.poll(() => api.bookmarks.find((b) => b.id === bm.id)?.is_archived).toBe(true);
    await expect(app.getByText('All-in on SQLite #litestream')).not.toBeVisible();

    // Switch to Archive tab and verify presence
    await app.getByTestId('tab-archive').click();
    await expect(app.getByText('All-in on SQLite #litestream')).toBeVisible();
  });

  test('opens Reader View modal and displays distraction-free article text', async ({ app, api }) => {
    api.seedBookmark({
      url: 'https://paulgraham.com/greatwork.html',
      title: 'How to Do Great Work',
      author: 'Paul Graham',
      domain: 'paulgraham.com',
      reader_content: 'If you wanted to collect all the advice on how to do great work into one rule, it would be: work on what you find interesting.\n\nGreat work usually entails spending a disproportionate amount of time on a problem.',
      reading_time_minutes: 25,
      tags: 'essays',
    });

    await gotoBookmarks(app);
    await expect(app.getByText('How to Do Great Work')).toBeVisible();

    // Click title to open reader modal
    await app.getByText('How to Do Great Work').click();

    const readerModal = app.getByTestId('reader-modal');
    await expect(readerModal).toBeVisible();
    await expect(readerModal.getByRole('heading', { name: 'How to Do Great Work' })).toBeVisible();
    await expect(readerModal.getByText(/By Paul Graham/i)).toBeVisible();
    await expect(readerModal.getByText(/work on what you find interesting/i)).toBeVisible();
    await expect(readerModal.getByText(/spending a disproportionate amount of time/i)).toBeVisible();

    // Test font size toggle buttons
    await readerModal.getByTitle('Large text').click();
    await readerModal.getByTitle('Small text').click();
    await readerModal.getByTitle('Default text').click();

    // Close reader modal
    await readerModal.getByRole('button', { name: /Close reader view/i }).click();
    await expect(readerModal).not.toBeVisible();
  });

  test('filters bookmarks by search query and tag chips', async ({ app, api }) => {
    api.seedBookmark({
      url: 'https://react.dev',
      title: 'React Documentation',
      tags: 'frontend,javascript',
    });
    api.seedBookmark({
      url: 'https://kernel.org',
      title: 'The Linux Kernel Archives',
      tags: 'linux,c',
    });

    await gotoBookmarks(app);
    await expect(app.getByText('React Documentation')).toBeVisible();
    await expect(app.getByText('The Linux Kernel Archives')).toBeVisible();

    // Filter by tag
    await app.getByTestId('tag-filters').getByRole('button', { name: '#frontend' }).click();
    await expect(app.getByText('React Documentation')).toBeVisible();
    await expect(app.getByText('The Linux Kernel Archives')).not.toBeVisible();

    // Clear tag
    await app.getByTestId('tag-filters').getByRole('button', { name: '#frontend' }).click();
    await expect(app.getByText('The Linux Kernel Archives')).toBeVisible();

    // Search input
    await app.getByPlaceholder(/Search title, URL, or tags/i).fill('Linux');
    await expect(app.getByText('The Linux Kernel Archives')).toBeVisible();
    await expect(app.getByText('React Documentation')).not.toBeVisible();
  });

  test('deletes a bookmark after window confirmation', async ({ app, api }) => {
    const bm = api.seedBookmark({
      url: 'https://temporary.example.com',
      title: 'Temporary Article to Delete',
      tags: 'temp',
    });

    await gotoBookmarks(app);
    await expect(app.getByText('Temporary Article to Delete')).toBeVisible();

    // Auto-accept confirmation dialog
    app.on('dialog', async (dialog) => {
      await dialog.accept();
    });

    const card = app.getByTestId(`bookmark-card-${bm.id}`);
    await card.getByTitle('Delete bookmark').click();

    await expect(app.getByText('Temporary Article to Delete')).not.toBeVisible();
    expect(api.bookmarks.find((b) => b.id === bm.id)).toBeUndefined();
  });
});
