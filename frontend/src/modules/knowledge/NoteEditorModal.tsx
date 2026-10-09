import React, { useState, useEffect, useRef } from 'react';
import { X, Eye, Edit3 } from 'lucide-react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { ApiHttpError } from '../../core/apiClient';
import { NOTE_TYPES } from './types';
import type { Note, NoteTypeValue } from './types';

export interface NoteEditorSave {
  title: string;
  content: string;
  noteType: NoteTypeValue;
  tags: string[];
  /** The body hash the draft was started from; null for a new note. */
  expectedContentHash: string | null;
}

interface NoteEditorModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (data: NoteEditorSave) => Promise<void>;
  /** The loaded note when editing; null or absent when creating. */
  initialNote?: Note | null;
  /** Editing, but the note's body has not arrived yet. */
  isLoading?: boolean;
  loadError?: string | null;
  /** Fetches the server's current copy, for resolving an edit conflict. */
  onReload?: () => Promise<Note | null>;
  isSaving: boolean;
}

const CONFLICT_MESSAGE = 'This note changed elsewhere — reload';

export const NoteEditorModal: React.FC<NoteEditorModalProps> = ({
  isOpen,
  onClose,
  onSave,
  initialNote,
  isLoading = false,
  loadError = null,
  onReload,
  isSaving,
}) => {
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [tagsInput, setTagsInput] = useState('');
  const [noteType, setNoteType] = useState<NoteTypeValue>('note');
  const [baseHash, setBaseHash] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [isReloading, setIsReloading] = useState(false);
  const [previewTab, setPreviewTab] = useState<'write' | 'preview'>('write');
  const [error, setError] = useState<string | null>(null);

  const titleInputRef = useRef<HTMLInputElement>(null);
  const isEditing = Boolean(initialNote) || isLoading || loadError !== null;
  const awaitingNote = isEditing && !initialNote;

  const applyNote = (note: Note | null) => {
    setTitle(note?.title ?? '');
    setContent(note?.content ?? '');
    setTagsInput(note ? note.tags.map((t) => `#${t}`).join(' ') : '');
    setNoteType((note?.note_type as NoteTypeValue | undefined) ?? 'note');
    setBaseHash(note?.content_hash ?? null);
    setConflict(false);
    setError(null);
  };

  // Seeds the draft when the editor opens or a different note arrives. It is
  // keyed on the note's id, not its identity: a background refetch of the same
  // note must not overwrite what the user is typing, and the hash the draft
  // started from has to stay the one that was loaded.
  //
  // The title is focused right here, not on a timer: the inputs are mounted in
  // the same commit, and a delayed focus() fires after the user has moved on
  // and steals the keystrokes meant for the field they are in.
  useEffect(() => {
    if (isOpen) {
      applyNote(initialNote ?? null);
      setPreviewTab('write');
      titleInputRef.current?.focus();
    }
  }, [isOpen, initialNote?.id]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen) return;
      if (e.key === 'Escape') {
        onClose();
      } else if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        handleSubmit();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  });

  if (!isOpen) return null;

  const handleSubmit = async () => {
    if (awaitingNote) return;
    const trimmedTitle = title.trim();

    if (!trimmedTitle) {
      setError('Title cannot be empty');
      return;
    }
    if (!content.trim()) {
      setError('Content cannot be empty');
      return;
    }

    const tags = tagsInput
      .split(/[\s,]+/)
      .map((t) => t.trim().replace(/^#+/, '').toLowerCase())
      .filter((t) => t.length > 0);

    setError(null);
    try {
      await onSave({
        title: trimmedTitle,
        // Sent as typed: trimming would change the stored hash (and trigger a
        // re-embed) for a note whose Markdown legitimately ends in a newline.
        content,
        noteType,
        tags,
        expectedContentHash: baseHash,
      });
      onClose();
    } catch (err: any) {
      if (err instanceof ApiHttpError && err.status === 409) {
        // Keep the draft on screen: the user decides whether to reload.
        setConflict(true);
        setError(null);
      } else {
        setError(err?.message || 'Failed to save note');
      }
    }
  };

  const handleReload = async () => {
    if (!onReload) return;
    setIsReloading(true);
    try {
      const fresh = await onReload();
      if (fresh) applyNote(fresh);
    } catch (err: any) {
      setError(err?.message || 'Failed to reload note');
    } finally {
      setIsReloading(false);
    }
  };

  const renderedPreview = () => {
    const html = marked.parse(content || '*Nothing to preview*') as string;
    const cleanHtml = DOMPurify.sanitize(html);
    return (
      <div
        className="prose dark:prose-invert max-w-none text-sm leading-relaxed"
        dangerouslySetInnerHTML={{ __html: cleanHtml }}
      />
    );
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="note-modal-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs animate-in fade-in duration-150"
    >
      <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-xl shadow-2xl w-full max-w-2xl max-h-[90vh] flex flex-col overflow-hidden">
        {/* Modal Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-neutral-200 dark:border-neutral-800">
          <h2
            id="note-modal-title"
            className="text-base font-semibold text-neutral-900 dark:text-neutral-100"
          >
            {isEditing ? 'Edit Note' : 'New Note'}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-1 rounded-md text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Modal Body */}
        <div className="p-6 overflow-y-auto space-y-4 flex-1">
          {awaitingNote ? (
            <div className="py-12 text-center text-xs text-neutral-400 font-mono">
              {loadError ?? 'Loading note...'}
            </div>
          ) : (
            <>
              {conflict && (
                <div
                  role="alert"
                  className="flex items-center justify-between gap-3 p-3 text-xs bg-amber-50 dark:bg-amber-950/40 text-amber-800 dark:text-amber-300 border border-amber-200 dark:border-amber-900 rounded-md"
                >
                  <span>{CONFLICT_MESSAGE}</span>
                  <button
                    type="button"
                    onClick={handleReload}
                    disabled={isReloading}
                    className="shrink-0 px-2.5 py-1 font-medium rounded-md border border-amber-300 dark:border-amber-800 hover:bg-amber-100 dark:hover:bg-amber-900/40 transition-colors disabled:opacity-50"
                  >
                    {isReloading ? 'Reloading...' : 'Reload'}
                  </button>
                </div>
              )}

              {error && (
                <div className="p-3 text-xs bg-rose-50 dark:bg-rose-950/40 text-rose-600 dark:text-rose-400 border border-rose-200 dark:border-rose-900 rounded-md">
                  {error}
                </div>
              )}

              {/* Title */}
              <div>
                <label
                  htmlFor="note-title-input"
                  className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1"
                >
                  Title
                </label>
                <input
                  id="note-title-input"
                  ref={titleInputRef}
                  type="text"
                  maxLength={200}
                  placeholder="Note title or core insight..."
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  className="w-full px-3 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
                />
              </div>

              {/* Type and Tags row */}
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label
                    htmlFor="note-type-select"
                    className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1"
                  >
                    Type
                  </label>
                  <select
                    id="note-type-select"
                    value={noteType}
                    onChange={(e) => setNoteType(e.target.value as NoteTypeValue)}
                    className="w-full px-3 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
                  >
                    {NOTE_TYPES.map((type) => (
                      <option key={type.value} value={type.value}>
                        {type.label}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label
                    htmlFor="note-tags-input"
                    className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1"
                  >
                    Tags (space or comma separated)
                  </label>
                  <input
                    id="note-tags-input"
                    type="text"
                    placeholder="#ideas #research #rust"
                    value={tagsInput}
                    onChange={(e) => setTagsInput(e.target.value)}
                    className="w-full px-3 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 font-mono transition"
                  />
                </div>
              </div>

              {/* Content with Edit / Preview Switch */}
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label
                    htmlFor="note-content-input"
                    className="block text-xs font-medium text-neutral-700 dark:text-neutral-300"
                  >
                    Content
                  </label>
                  <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-800 p-0.5 rounded-md">
                    <button
                      type="button"
                      onClick={() => setPreviewTab('write')}
                      className={`flex items-center gap-1 px-2.5 py-1 text-xs rounded transition-colors ${
                        previewTab === 'write'
                          ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs font-medium'
                          : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                      }`}
                    >
                      <Edit3 className="w-3 h-3" />
                      <span>Write</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => setPreviewTab('preview')}
                      className={`flex items-center gap-1 px-2.5 py-1 text-xs rounded transition-colors ${
                        previewTab === 'preview'
                          ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs font-medium'
                          : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                      }`}
                    >
                      <Eye className="w-3 h-3" />
                      <span>Preview</span>
                    </button>
                  </div>
                </div>

                {previewTab === 'write' ? (
                  <textarea
                    id="note-content-input"
                    rows={10}
                    placeholder="Write your note in markdown..."
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    className="w-full px-3 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 font-mono transition resize-y"
                  />
                ) : (
                  <div className="w-full min-h-[240px] max-h-[360px] p-3 overflow-y-auto bg-neutral-50 dark:bg-neutral-800/60 border border-neutral-300 dark:border-neutral-700 rounded-md">
                    {renderedPreview()}
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        {/* Modal Footer */}
        <div className="flex items-center justify-between px-6 py-4 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/50">
          <span className="text-xs text-neutral-400 font-mono hidden sm:inline">
            Ctrl+Enter to save
          </span>
          <div className="flex items-center gap-2 ml-auto">
            <button
              type="button"
              onClick={onClose}
              className="px-3.5 py-1.5 text-xs font-medium rounded-md text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSubmit}
              disabled={isSaving || awaitingNote || !title.trim() || !content.trim()}
              className="px-4 py-1.5 text-xs font-medium rounded-md bg-neutral-900 text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isSaving ? 'Saving...' : isEditing ? 'Update Note' : 'Create Note'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
