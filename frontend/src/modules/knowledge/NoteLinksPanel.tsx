import React from 'react';
import type { NoteLink, NoteLinks } from './types';

interface NoteLinksPanelProps {
  links: NoteLinks | undefined;
  isLoading: boolean;
  onOpenNote: (id: number) => void;
}

const LINK_BUTTON =
  'text-left text-xs text-neutral-700 dark:text-neutral-300 underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-neutral-400 transition-colors';

function LinkList({
  heading,
  items,
  onOpenNote,
  emptyText,
}: {
  heading: string;
  items: NoteLink[];
  onOpenNote: (id: number) => void;
  emptyText: string;
}) {
  return (
    <div className="space-y-1.5">
      <h3 className="text-xs font-medium text-neutral-700 dark:text-neutral-300">{heading}</h3>
      {items.length === 0 ? (
        <p className="text-xs text-neutral-400">{emptyText}</p>
      ) : (
        <ul className="space-y-1">
          {items.map((link) => (
            <li key={`${link.note_id ?? 'ghost'}-${link.title}`}>
              {link.note_id === null ? (
                <span className="text-xs text-neutral-400" title="No note has this title yet">
                  {link.title} (no note yet)
                </span>
              ) : (
                <button
                  type="button"
                  className={LINK_BUTTON}
                  onClick={() => onOpenNote(link.note_id as number)}
                >
                  {link.title}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Backlinks and outgoing links of a saved note. */
export const NoteLinksPanel: React.FC<NoteLinksPanelProps> = ({ links, isLoading, onOpenNote }) => {
  if (isLoading) {
    return <p className="text-xs text-neutral-400 font-mono">Loading links...</p>;
  }
  if (!links) return null;
  return (
    <section
      aria-label="Linked notes"
      data-testid="note-links-panel"
      className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-4 border-t border-neutral-200 dark:border-neutral-800"
    >
      <LinkList
        heading="Mentioned in"
        items={links.backlinks}
        onOpenNote={onOpenNote}
        emptyText="No notes link here yet."
      />
      <LinkList
        heading="Links to"
        items={links.outgoing}
        onOpenNote={onOpenNote}
        emptyText="Use [[Note Title]] to link notes."
      />
    </section>
  );
};
