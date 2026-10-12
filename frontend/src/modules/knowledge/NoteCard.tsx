import React from 'react';
import { Edit2, Archive, ArchiveRestore, Trash2, FileText } from 'lucide-react';
import { formatInZone, useSettings } from '../../core/settings';
import type { NoteSummary } from './types';

interface NoteCardProps {
  note: NoteSummary;
  onEdit: (note: NoteSummary) => void;
  onDelete: (note: NoteSummary) => void;
  onToggleArchive: (note: NoteSummary) => void;
  onSelectTag?: (tag: string) => void;
}

export const NoteCard: React.FC<NoteCardProps> = ({
  note,
  onEdit,
  onDelete,
  onToggleArchive,
  onSelectTag,
}) => {
  const { timezone } = useSettings();
  const formattedDate = formatInZone(note.updated_at, timezone, {
    month: 'short',
    day: 'numeric',
  });

  return (
    <article
      data-testid={`note-card-${note.id}`}
      className="group relative flex flex-col justify-between rounded-xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 hover:border-neutral-400 dark:hover:border-neutral-600 transition-colors p-5 space-y-3 shadow-xs"
    >
      <div className="space-y-2">
        {/* Top Header: Type & Action Buttons */}
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5 text-neutral-500">
            <FileText className="w-3.5 h-3.5 text-neutral-400" />
            <span className="text-[11px] font-mono uppercase tracking-wider text-neutral-400">
              {note.note_type}
            </span>
            {note.source_channel && (
              <>
                <span className="text-neutral-300 dark:text-neutral-700">·</span>
                <span className="text-[11px] font-mono text-neutral-400">
                  {note.source_channel}
                </span>
              </>
            )}
          </div>

          <div className="flex items-center gap-1 opacity-70 group-hover:opacity-100 transition-opacity">
            <button
              type="button"
              onClick={() => onEdit(note)}
              aria-label="Edit note"
              className="p-1 rounded-md text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
            >
              <Edit2 className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => onToggleArchive(note)}
              aria-label={note.is_archived ? 'Restore note' : 'Archive note'}
              className="p-1 rounded-md text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
            >
              {note.is_archived ? (
                <ArchiveRestore className="w-3.5 h-3.5" />
              ) : (
                <Archive className="w-3.5 h-3.5" />
              )}
            </button>
            <button
              type="button"
              onClick={() => onDelete(note)}
              aria-label="Delete note"
              className="p-1 rounded-md text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/40 transition-colors"
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Title */}
        <h3
          onClick={() => onEdit(note)}
          className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 leading-snug cursor-pointer hover:underline decoration-neutral-400 underline-offset-2"
        >
          {note.title}
        </h3>

        {/* Excerpt */}
        <p className="text-xs text-neutral-600 dark:text-neutral-400 line-clamp-3 leading-relaxed font-sans">
          {note.excerpt}
        </p>
      </div>

      {/* Footer: Tags and Timestamp */}
      <div className="pt-2 border-t border-neutral-100 dark:border-neutral-800/80 flex items-center justify-between gap-2 text-xs">
        <div className="flex items-center gap-1.5 flex-wrap min-w-0">
          {note.tags.map((tag) => (
            <button
              key={tag}
              type="button"
              onClick={() => onSelectTag?.(tag)}
              className="font-mono text-[11px] px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700 transition-colors truncate"
            >
              #{tag}
            </button>
          ))}
        </div>
        <span className="text-[11px] text-neutral-400 font-mono shrink-0">
          {formattedDate}
        </span>
      </div>
    </article>
  );
};
