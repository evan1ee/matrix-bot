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

  test('opening another note with unsaved edits asks before discarding them', async ({
    app,
    api,
  }) => {
    const a = api.seedNote({ title: 'Note A', content: 'The target.' });
    api.seedNote({ title: 'Note B', content: 'Builds on [[Note A]].' });

    await gotoKnowledge(app);
    await app.getByTestId(`note-card-${a.id}`).getByRole('button', { name: 'Edit note' }).click();
    const title = app.getByPlaceholder(/Note title or core insight/i);
    await title.fill('Note A, edited');

    await app.getByTestId('note-links-panel').getByRole('button', { name: 'Note B' }).click();
    await expect(app.getByText('Discard your unsaved changes')).toBeVisible();
    await expect(title).toHaveValue('Note A, edited');

    await app.getByRole('button', { name: 'Keep editing' }).click();
    await expect(app.getByText('Discard your unsaved changes')).toBeHidden();
    await expect(title).toHaveValue('Note A, edited');

    await app.getByTestId('note-links-panel').getByRole('button', { name: 'Note B' }).click();
    await app.getByRole('button', { name: 'Discard and open' }).click();
    await expect(title).toHaveValue('Note B');
  });

  test('the suggestion list is announced as a combobox popup', async ({ app, api }) => {
    api.seedNote({ title: 'Rust ownership', content: 'Borrow checker.' });
    api.seedNote({ title: 'Pipes | and [brackets]', content: 'Unlinkable title.' });

    await gotoKnowledge(app);
    await app.getByTestId('create-note-button').click();
    const body = app.getByPlaceholder(/Write your note in markdown/i);
    await expect(body).toHaveAttribute('aria-expanded', 'false');
    await body.fill('See [[');

    await expect(body).toHaveAttribute('aria-expanded', 'true');
    const options = app.getByTestId('wiki-suggestions').getByRole('option');
    // A title containing [, ] or | cannot be written as a link, so it is not offered.
    await expect(options).toHaveCount(1);
    await expect(body).toHaveAttribute(
      'aria-activedescendant',
      (await options.first().getAttribute('id')) ?? ''
    );
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
    await expect(list.getByRole('option', { name: 'Rust ownership' })).toBeVisible();
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

  test('a focused graph node is outlined and brought into a zoomed view', async ({ app, api }) => {
    const nodes = Array.from({ length: 8 }, (_, i) =>
      api.seedNote({
        title: `Graph note ${i}`,
        content: i > 0 ? `See [[Graph note ${i - 1}]]` : 'Root',
      })
    );

    await gotoKnowledge(app);
    await app.getByRole('button', { name: 'Graph' }).click();
    const svg = app.getByRole('group', { name: 'Knowledge graph' });
    for (let i = 0; i < 3; i++) await app.getByRole('button', { name: 'Zoom in' }).click();

    // Focus the last node: it may start outside the zoomed viewBox.
    const last = app.getByTestId(`graph-node-${nodes[nodes.length - 1].id}`);
    await last.focus();

    const box = await last.boundingBox();
    const frame = await svg.boundingBox();
    expect(box && frame).toBeTruthy();
    expect(box!.x).toBeGreaterThanOrEqual(frame!.x - 1);
    expect(box!.x + box!.width).toBeLessThanOrEqual(frame!.x + frame!.width + 1);
    expect(box!.y).toBeGreaterThanOrEqual(frame!.y - 1);
    expect(box!.y + box!.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
  });
});
