import React, { useState, useEffect, useRef } from 'react';
import { X, Eye, Edit3 } from 'lucide-react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import type { Note, CreateNoteInput } from './types';

interface NoteEditorModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (data: { title: string; content: string; format: string; tags: string[] }) => Promise<void>;
  initialNote?: Note | null;
  isSaving: boolean;
}

export const NoteEditorModal: React.FC<NoteEditorModalProps> = ({
  isOpen,
  onClose,
  onSave,
  initialNote,
  isSaving,
}) => {
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [tagsInput, setTagsInput] = useState('');
  const [format, setFormat] = useState<'markdown' | 'plain' | 'quote'>('markdown');
  const [previewTab, setPreviewTab] = useState<'write' | 'preview'>('write');
  const [error, setError] = useState<string | null>(null);

  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isOpen) {
      if (initialNote) {
        setTitle(initialNote.title);
        setContent(initialNote.content);
        setTagsInput(initialNote.tags.map((t) => `#${t}`).join(' '));
        setFormat((initialNote.format as 'markdown' | 'plain' | 'quote') || 'markdown');
      } else {
        setTitle('');
        setContent('');
        setTagsInput('');
        setFormat('markdown');
      }
      setPreviewTab('write');
      setError(null);
      setTimeout(() => titleInputRef.current?.focus(), 50);
    }
  }, [isOpen, initialNote]);

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
    const trimmedTitle = title.trim();
    const trimmedContent = content.trim();

    if (!trimmedTitle) {
      setError('Title cannot be empty');
      return;
    }
    if (!trimmedContent) {
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
        content: trimmedContent,
        format,
        tags,
      });
      onClose();
    } catch (err: any) {
      setError(err?.message || 'Failed to save note');
    }
  };

  const renderedPreview = () => {
    if (format === 'plain') {
      return (
        <pre className="whitespace-pre-wrap font-mono text-sm text-neutral-800 dark:text-neutral-200">
          {content || 'Nothing to preview'}
        </pre>
      );
    }
    if (format === 'quote') {
      return (
        <blockquote className="border-l-4 border-neutral-400 dark:border-neutral-600 pl-4 italic text-neutral-700 dark:text-neutral-300">
          {content || 'Nothing to preview'}
        </blockquote>
      );
    }
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
            {initialNote ? 'Edit Note' : 'New Note'}
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

          {/* Format and Tags row */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div>
              <label
                htmlFor="note-format-select"
                className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1"
              >
                Format
              </label>
              <select
                id="note-format-select"
                value={format}
                onChange={(e) => setFormat(e.target.value as any)}
                className="w-full px-3 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
              >
                <option value="markdown">Markdown</option>
                <option value="plain">Plain text</option>
                <option value="quote">Quote snippet</option>
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
              disabled={isSaving || !title.trim() || !content.trim()}
              className="px-4 py-1.5 text-xs font-medium rounded-md bg-neutral-900 text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isSaving ? 'Saving...' : initialNote ? 'Update Note' : 'Create Note'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
