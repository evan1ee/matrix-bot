import { test, expect, gotoApp, TAB } from './fixtures';

const gotoKnowledge = (page: import('@playwright/test').Page) => gotoApp(page, TAB.knowledge);

test.describe('Note wiki-links', () => {
  test('shows a backlink in the linked note and opens the linking note from it', async ({
    app,
    api,
  }) => {
    const a = api.seedNote({ title: 'Note A', content: 'The target.' });
    const b = api.seedNote({ title: 'Note B', content: 'Builds on [[Note A]].' });

    await gotoKnowledge(app);
    await app.getByTestId(`note-card-${a.id}`).getByRole('button', { name: 'Edit note' }).click();

    const panel = app.getByTestId('note-links-panel');
    await expect(panel.getByRole('button', { name: 'Note B' })).toBeVisible();

    await panel.getByRole('button', { name: 'Note B' }).click();
    await expect(app.getByPlaceholder(/Note title or core insight/i)).toHaveValue('Note B');
    await expect(
      app.getByTestId('note-links-panel').getByRole('button', { name: 'Note A' })
    ).toBeVisible();
    expect(b.id).not.toBe(a.id);
  });

  test('lists a link to a missing note as not created yet', async ({ app, api }) => {
    const note = api.seedNote({ title: 'Lonely', content: 'See [[Ghost note]].' });

    await gotoKnowledge(app);
    await app.getByTestId(`note-card-${note.id}`).getByRole('button', { name: 'Edit note' }).click();

    await expect(app.getByTestId('note-links-panel')).toContainText('Ghost note (no note yet)');
  });

  test('completes a [[ link from the title suggestions', async ({ app, api }) => {
    api.seedNote({ title: 'Rust ownership', content: 'Borrow checker.' });

    await gotoKnowledge(app);
    await app.getByTestId('create-note-button').click();
    await app.getByPlaceholder(/Note title or core insight/i).fill('Index');
    const body = app.getByPlaceholder(/Write your note in markdown/i);
    await body.fill('See [[Rus');

    const list = app.getByTestId('wiki-suggestions');
    await expect(list.getByRole('button', { name: 'Rust ownership' })).toBeVisible();
    await body.press('Enter');

    await expect(body).toHaveValue('See [[Rust ownership]]');
    await expect(list).toBeHidden();
  });

  test('Escape closes the suggestions without closing the editor', async ({ app, api }) => {
    api.seedNote({ title: 'Rust ownership', content: 'Borrow checker.' });

    await gotoKnowledge(app);
    await app.getByTestId('create-note-button').click();
    const body = app.getByPlaceholder(/Write your note in markdown/i);
    await body.fill('[[');
    await expect(app.getByTestId('wiki-suggestions')).toBeVisible();

    await body.press('Escape');
    await expect(app.getByTestId('wiki-suggestions')).toBeHidden();
    await expect(app.getByRole('heading', { name: 'New Note' })).toBeVisible();
  });

  test('renders [[links]] in the preview and follows one', async ({ app, api }) => {
    const target = api.seedNote({ title: 'Target', content: 'Destination.' });
    const src = api.seedNote({ title: 'Source', content: 'Go to [[Target|the target]].' });

    await gotoKnowledge(app);
    await app.getByTestId(`note-card-${src.id}`).getByRole('button', { name: 'Edit note' }).click();
    await app.getByRole('button', { name: 'Preview' }).click();
    await app.getByRole('link', { name: 'the target' }).click();

    await expect(app.getByPlaceholder(/Note title or core insight/i)).toHaveValue('Target');
    expect(target.id).not.toBe(src.id);
  });

  test('graph tab shows notes and their link edges, and opens a note on click', async ({
    app,
    api,
  }) => {
    const a = api.seedNote({ title: 'Alpha', content: 'Links to [[Beta]].' });
    const b = api.seedNote({ title: 'Beta', content: 'Plain.' });
    api.seedNote({ title: 'Hidden', content: 'Archived.', is_archived: true });

    await gotoKnowledge(app);
    await app.getByRole('button', { name: 'Graph' }).click();

    await expect(app.getByTestId(`graph-node-${a.id}`)).toBeVisible();
    await expect(app.getByTestId(`graph-node-${b.id}`)).toBeVisible();
    await expect(app.getByTestId(`graph-edge-${a.id}-${b.id}`)).toBeAttached();
    await expect(app.getByText('Hidden')).toHaveCount(0);

    await app.getByTestId(`graph-node-${b.id}`).click();
    await expect(app.getByPlaceholder(/Note title or core insight/i)).toHaveValue('Beta');
  });
});
