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
});
