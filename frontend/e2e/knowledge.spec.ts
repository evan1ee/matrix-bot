import { test, expect, gotoApp, TAB } from './fixtures';

const gotoKnowledge = (page: import('@playwright/test').Page) => gotoApp(page, TAB.knowledge);

test.describe('Notes', () => {
  test('renders empty knowledge view with action buttons and empty state', async ({ app }) => {
    await gotoKnowledge(app);

    await expect(app.getByRole('heading', { name: 'Notes', exact: true })).toBeVisible();
    await expect(app.getByRole('button', { name: 'Active Notes' })).toBeVisible();
    await expect(app.getByRole('button', { name: 'Archive' })).toBeVisible();
    await expect(app.getByRole('button', { name: 'AI Search' })).toBeVisible();
    await expect(app.getByTestId('create-note-button')).toBeVisible();

    await expect(app.getByText('No notes yet')).toBeVisible();
  });

  test('creates a new note and displays it in the active notes grid', async ({ app, api }) => {
    await gotoKnowledge(app);

    await app.getByTestId('create-note-button').click();
    await expect(app.getByRole('heading', { name: 'New Note' })).toBeVisible();

    await app.getByPlaceholder(/Note title or core insight/i).fill('Rust Memory Model');
    await app.getByPlaceholder(/#ideas #research/i).fill('rust systems memory');
    await app.getByPlaceholder(/Write your note in markdown/i).fill('Ownership, borrowing, and lifetimes enforce compile-time safety without GC.');

    await app.getByRole('button', { name: 'Create Note' }).click();

    // Verify modal closes and card renders
    await expect(app.getByRole('heading', { name: 'New Note' })).not.toBeVisible();
    const card = app.getByTestId(/^note-card-\d+/).first();
    await expect(card).toBeVisible();
    await expect(card.getByText('Rust Memory Model')).toBeVisible();
    await expect(card.getByText('#rust')).toBeVisible();
    await expect(card.getByText('#systems')).toBeVisible();

    // Verify persisted in fake API
    const created = api.notes.find((n) => n.title === 'Rust Memory Model');
    expect(created).toBeDefined();
    expect(created?.tags).toContain('rust');
  });

  test('edits an existing note title and content', async ({ app, api }) => {
    const existing = api.seedNote({
      title: 'Initial Note Title',
      content: 'Original note body text.',
      tags: ['draft'],
    });

    await gotoKnowledge(app);

    const card = app.getByTestId(`note-card-${existing.id}`);
    await expect(card).toBeVisible();

    await card.getByRole('button', { name: 'Edit note' }).click();
    await expect(app.getByRole('heading', { name: 'Edit Note' })).toBeVisible();

    await app.getByPlaceholder(/Note title or core insight/i).fill('Updated Note Title');
    await app.getByRole('button', { name: 'Update Note' }).click();

    await expect(app.getByRole('heading', { name: 'Edit Note' })).not.toBeVisible();
    await expect(card.getByText('Updated Note Title')).toBeVisible();

    expect(api.notes.find((n) => n.id === existing.id)?.title).toBe('Updated Note Title');
  });

  test('filters notes by tag selection', async ({ app, api }) => {
    api.seedNote({ title: 'Rust Note', content: 'About borrow checker', tags: ['rust'] });
    api.seedNote({ title: 'Design Note', content: 'About typography', tags: ['design'] });

    await gotoKnowledge(app);

    await expect(app.getByText('Rust Note')).toBeVisible();
    await expect(app.getByText('Design Note')).toBeVisible();

    // Click #rust tag chip
    await app.getByTestId('tag-filter-chips').getByRole('button', { name: '#rust' }).click();

    await expect(app.getByText('Rust Note')).toBeVisible();
    await expect(app.getByText('Design Note')).not.toBeVisible();

    // Click All tag chip to clear filter
    await app.getByTestId('tag-filter-chips').getByRole('button', { name: 'All', exact: true }).click();
    await expect(app.getByText('Design Note')).toBeVisible();
  });

  test('deletes a note with confirmation modal', async ({ app, api }) => {
    const note = api.seedNote({ title: 'Note to be deleted', content: 'Ephemeral snippet' });

    await gotoKnowledge(app);

    const card = app.getByTestId(`note-card-${note.id}`);
    await expect(card).toBeVisible();

    await card.getByRole('button', { name: 'Delete note' }).click();
    await expect(app.getByRole('heading', { name: 'Delete Note' })).toBeVisible();

    await app.getByTestId('confirm-modal').getByRole('button', { name: 'Delete Note' }).click();
    await expect(card).not.toBeVisible();

    expect(api.notes.find((n) => n.id === note.id)).toBeUndefined();
  });

  test('searches knowledge with AI search panel', async ({ app, api }) => {
    api.seedNote({ title: 'Transformer Attention', content: 'Scaled dot-product attention explanation' });

    await gotoKnowledge(app);

    await app.getByRole('button', { name: 'AI Search' }).click();
    await expect(app.getByPlaceholder(/Semantic & hybrid search across notes/i)).toBeVisible();

    await app.getByPlaceholder(/Semantic & hybrid search across notes/i).fill('attention');

    await expect(app.getByText('Transformer Attention')).toBeVisible();
    await expect(app.getByText(/92% match/i)).toBeVisible();
  });

  test('closes note editor on Escape key', async ({ app }) => {
    await gotoKnowledge(app);

    await app.getByTestId('create-note-button').click();
    await expect(app.getByRole('heading', { name: 'New Note' })).toBeVisible();

    await app.keyboard.press('Escape');
    await expect(app.getByRole('heading', { name: 'New Note' })).not.toBeVisible();
  });

  test('lists an excerpt and fetches the body only when a note is opened', async ({ app, api }) => {
    const note = api.seedNote({
      title: 'Long note',
      content: `Opening words. ${'filler '.repeat(60)}TAILMARKER`,
    });
    const documentFetches: string[] = [];
    app.on('request', (req) => {
      if (new RegExp(`/api/v1/notes/${note.id}$`).test(req.url()) && req.method() === 'GET') {
        documentFetches.push(req.url());
      }
    });

    await gotoKnowledge(app);

    const card = app.getByTestId(`note-card-${note.id}`);
    await expect(card.getByText(/Opening words\./)).toBeVisible();
    await expect(card.getByText('TAILMARKER')).toHaveCount(0);
    expect(documentFetches).toHaveLength(0);

    await card.getByRole('button', { name: 'Edit note' }).click();
    await expect(app.getByLabel('Content')).toHaveValue(/TAILMARKER$/);
    expect(documentFetches).toHaveLength(1);
  });

  test('keeps the draft and offers a reload when the note changed elsewhere', async ({ app, api }) => {
    const note = api.seedNote({ title: 'Shared plan', content: 'Version one' });

    await gotoKnowledge(app);
    await app.getByTestId(`note-card-${note.id}`).getByRole('button', { name: 'Edit note' }).click();
    await expect(app.getByLabel('Content')).toHaveValue('Version one');

    await app.getByLabel('Content').fill('My unsaved draft');
    // Another device saves while this editor is open.
    note.content = 'Version two, from elsewhere';

    await app.getByRole('button', { name: 'Update Note' }).click();

    await expect(app.getByText('This note changed elsewhere — reload')).toBeVisible();
    await expect(app.getByRole('heading', { name: 'Edit Note' })).toBeVisible();
    await expect(app.getByLabel('Content')).toHaveValue('My unsaved draft');
    expect(note.content).toBe('Version two, from elsewhere');

    await app.getByRole('button', { name: 'Reload' }).click();
    await expect(app.getByLabel('Content')).toHaveValue('Version two, from elsewhere');
    await expect(app.getByText('This note changed elsewhere — reload')).toHaveCount(0);

    await app.getByLabel('Content').fill('Merged by hand');
    await app.getByRole('button', { name: 'Update Note' }).click();
    await expect(app.getByRole('heading', { name: 'Edit Note' })).not.toBeVisible();
    expect(note.content).toBe('Merged by hand');
  });

  test('saves the chosen note type', async ({ app, api }) => {
    await gotoKnowledge(app);

    await app.getByTestId('create-note-button').click();
    await app.getByPlaceholder(/Note title or core insight/i).fill('Cache reads');
    await app.getByLabel('Type').selectOption('idea');
    await app.getByPlaceholder(/Write your note in markdown/i).fill('A read-through cache would cut latency.');
    await app.getByRole('button', { name: 'Create Note' }).click();

    await expect(app.getByRole('heading', { name: 'New Note' })).not.toBeVisible();
    expect(api.notes.find((n) => n.title === 'Cache reads')?.note_type).toBe('idea');
    await expect(app.getByTestId(/^note-card-\d+/).first().getByText('idea', { exact: true })).toBeVisible();
  });

  test('says so when search fell back to keyword results', async ({ app, api }) => {
    api.seedNote({ title: 'Transformer Attention', content: 'Scaled dot-product attention' });
    api.searchMode = 'lexical';

    await gotoKnowledge(app);
    await app.getByRole('button', { name: 'AI Search' }).click();
    await app.getByPlaceholder(/Semantic & hybrid search across notes/i).fill('attention');

    await expect(app.getByText('Transformer Attention')).toBeVisible();
    await expect(app.getByTestId('search-lexical-notice')).toBeVisible();
    await expect(app.getByTestId('search-lexical-notice')).toContainText(/keyword/i);
  });

  test('shows no keyword notice when search is hybrid', async ({ app, api }) => {
    api.seedNote({ title: 'Transformer Attention', content: 'Scaled dot-product attention' });

    await gotoKnowledge(app);
    await app.getByRole('button', { name: 'AI Search' }).click();
    await app.getByPlaceholder(/Semantic & hybrid search across notes/i).fill('attention');

    await expect(app.getByText('Transformer Attention')).toBeVisible();
    await expect(app.getByTestId('search-lexical-notice')).toHaveCount(0);
  });
});
