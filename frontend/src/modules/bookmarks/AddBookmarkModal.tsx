import React, { useState, useEffect } from 'react';
import { X, Link as LinkIcon, Loader2 } from 'lucide-react';
import type { CreateBookmarkInput } from './types';

interface AddBookmarkModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (payload: CreateBookmarkInput) => Promise<void>;
}

export const AddBookmarkModal: React.FC<AddBookmarkModalProps> = ({
  isOpen,
  onClose,
  onSave,
}) => {
  const [url, setUrl] = useState('');
  const [title, setTitle] = useState('');
  const [tagsInput, setTagsInput] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      setUrl('');
      setTitle('');
      setTagsInput('');
      setError(null);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmedUrl = url.trim();
    if (!trimmedUrl) {
      setError('Please enter a valid URL.');
      return;
    }

    if (!trimmedUrl.startsWith('http://') && !trimmedUrl.startsWith('https://')) {
      setError('URL must start with http:// or https://');
      return;
    }

    setIsSubmitting(true);
    setError(null);

    const parsedTags = tagsInput
      .split(',')
      .map((t) => t.trim().toLowerCase())
      .filter(Boolean);

    try {
      await onSave({
        url: trimmedUrl,
        title: title.trim() || undefined,
        tags: parsedTags.length > 0 ? parsedTags : undefined,
      });
      onClose();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Failed to save bookmark';
      setError(msg);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="add-bookmark-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
    >
      <div
        data-testid="add-bookmark-modal"
        className="w-full max-w-md rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-6 shadow-xl space-y-5"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <LinkIcon className="w-4 h-4 text-neutral-500" />
            <h3
              id="add-bookmark-title"
              className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans"
            >
              Save New Bookmark
            </h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close dialog"
            className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {error && (
          <div className="p-2.5 rounded text-xs bg-red-500/10 text-red-600 dark:text-red-400 border border-red-500/20 font-sans">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* URL */}
          <div className="space-y-1.5">
            <label
              htmlFor="bookmark-url"
              className="text-[10px] font-mono uppercase tracking-wider text-neutral-500"
            >
              Article or Web Page URL *
            </label>
            <input
              id="bookmark-url"
              type="url"
              required
              autoFocus
              placeholder="https://example.com/article"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          {/* Optional Title */}
          <div className="space-y-1.5">
            <label
              htmlFor="bookmark-title"
              className="text-[10px] font-mono uppercase tracking-wider text-neutral-500"
            >
              Custom Title (Optional)
            </label>
            <input
              id="bookmark-title"
              type="text"
              maxLength={200}
              placeholder="Leave blank to auto-extract title from page"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          {/* Tags */}
          <div className="space-y-1.5">
            <label
              htmlFor="bookmark-tags"
              className="text-[10px] font-mono uppercase tracking-wider text-neutral-500"
            >
              Tags (Comma separated)
            </label>
            <input
              id="bookmark-tags"
              type="text"
              placeholder="tech, reading, rust"
              value={tagsInput}
              onChange={(e) => setTagsInput(e.target.value)}
              className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3.5 text-xs rounded-md border border-neutral-300 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-700 dark:text-neutral-300 transition cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="h-9 inline-flex items-center gap-1.5 px-3.5 rounded-md bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-neutral-100 dark:hover:bg-neutral-200 dark:text-neutral-900 text-xs font-medium transition disabled:opacity-50 cursor-pointer"
            >
              {isSubmitting && <Loader2 className="w-3 h-3 animate-spin" />}
              <span>{isSubmitting ? 'Fetching & Saving...' : 'Save Bookmark'}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
