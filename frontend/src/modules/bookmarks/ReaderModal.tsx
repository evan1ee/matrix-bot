import React, { useState, useEffect, useRef } from 'react';
import { X, ExternalLink, Star, Archive, ArchiveRestore } from 'lucide-react';
import type { Bookmark } from './types';

interface ReaderModalProps {
  bookmark: Bookmark | null;
  isOpen: boolean;
  onClose: () => void;
  onToggleFavorite: (id: number) => void;
  onToggleArchive: (id: number) => void;
}

export const ReaderModal: React.FC<ReaderModalProps> = ({
  bookmark,
  isOpen,
  onClose,
  onToggleFavorite,
  onToggleArchive,
}) => {
  const [fontSize, setFontSize] = useState<'sm' | 'base' | 'lg'>('base');
  const [scrollProgress, setScrollProgress] = useState(0);
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    if (isOpen) {
      window.addEventListener('keydown', handleKeyDown);
      setScrollProgress(0);
    }
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  const handleScroll = () => {
    if (!scrollContainerRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = scrollContainerRef.current;
    const total = scrollHeight - clientHeight;
    if (total <= 0) {
      setScrollProgress(100);
    } else {
      setScrollProgress(Math.min(100, Math.max(0, (scrollTop / total) * 100)));
    }
  };

  if (!isOpen || !bookmark) return null;

  const paragraphs = bookmark.reader_content
    ? bookmark.reader_content
        .split(/\n\n+/)
        .map((p) => p.trim())
        .filter(Boolean)
    : [];

  const textSizeClass =
    fontSize === 'sm'
      ? 'text-sm leading-relaxed'
      : fontSize === 'lg'
        ? 'text-lg leading-relaxed'
        : 'text-base leading-relaxed';

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="reader-modal-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-2 sm:p-4 bg-black/60 backdrop-blur-xs"
    >
      <div
        data-testid="reader-modal"
        className="relative flex flex-col w-full max-w-3xl h-[92vh] rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden"
      >
        {/* Scroll reading progress bar */}
        <div className="w-full h-0.5 bg-neutral-100 dark:bg-neutral-900 shrink-0">
          <div
            className="h-full bg-neutral-900 dark:bg-neutral-100 transition-all duration-75"
            style={{ width: `${scrollProgress}%` }}
          />
        </div>

        {/* Top Control Bar */}
        <header className="flex items-center justify-between px-4 sm:px-6 py-3 border-b border-neutral-200 dark:border-neutral-800 bg-white/95 dark:bg-neutral-950/95 shrink-0">
          <div className="flex items-center gap-2 min-w-0 pr-4">
            <span className="text-[11px] font-mono uppercase tracking-wider text-neutral-500 truncate">
              {bookmark.domain}
            </span>
            {bookmark.reading_time_minutes > 0 && (
              <>
                <span className="text-neutral-300 dark:text-neutral-700">·</span>
                <span className="text-[11px] font-mono text-neutral-500 shrink-0">
                  {bookmark.reading_time_minutes} min read
                </span>
              </>
            )}
          </div>

          <div className="flex items-center gap-1.5 shrink-0">
            {/* Font size toggles */}
            <div className="hidden sm:flex items-center border border-neutral-200 dark:border-neutral-800 rounded px-1 py-0.5 mr-2">
              <button
                type="button"
                onClick={() => setFontSize('sm')}
                className={`px-1.5 py-0.5 text-xs font-mono rounded cursor-pointer transition ${
                  fontSize === 'sm'
                    ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-semibold'
                    : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                }`}
                title="Small text"
              >
                A-
              </button>
              <button
                type="button"
                onClick={() => setFontSize('base')}
                className={`px-1.5 py-0.5 text-xs font-mono rounded cursor-pointer transition ${
                  fontSize === 'base'
                    ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-semibold'
                    : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                }`}
                title="Default text"
              >
                A
              </button>
              <button
                type="button"
                onClick={() => setFontSize('lg')}
                className={`px-1.5 py-0.5 text-xs font-mono rounded cursor-pointer transition ${
                  fontSize === 'lg'
                    ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-semibold'
                    : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                }`}
                title="Large text"
              >
                A+
              </button>
            </div>

            {/* Favorite toggle */}
            <button
              type="button"
              onClick={() => onToggleFavorite(bookmark.id)}
              aria-label={bookmark.is_favorite ? 'Unfavorite bookmark' : 'Favorite bookmark'}
              className={`p-1.5 rounded transition cursor-pointer ${
                bookmark.is_favorite
                  ? 'text-amber-500 hover:text-amber-600 bg-amber-50 dark:bg-amber-950/30'
                  : 'text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-900'
              }`}
            >
              <Star
                className={`w-4 h-4 ${bookmark.is_favorite ? 'fill-amber-500' : ''}`}
              />
            </button>

            {/* Archive toggle */}
            <button
              type="button"
              onClick={() => onToggleArchive(bookmark.id)}
              aria-label={bookmark.is_archived ? 'Move to unread' : 'Archive bookmark'}
              className="p-1.5 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer"
            >
              {bookmark.is_archived ? (
                <ArchiveRestore className="w-4 h-4" />
              ) : (
                <Archive className="w-4 h-4" />
              )}
            </button>

            {/* Original external link */}
            <a
              href={bookmark.url}
              target="_blank"
              rel="noopener noreferrer"
              aria-label="Open original link"
              className="p-1.5 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer"
            >
              <ExternalLink className="w-4 h-4" />
            </a>

            {/* Close */}
            <button
              type="button"
              onClick={onClose}
              aria-label="Close reader view"
              className="p-1.5 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer ml-1"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </header>

        {/* Scrollable Reader Content */}
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          className="flex-1 overflow-y-auto px-6 sm:px-12 py-8 bg-white dark:bg-neutral-950"
        >
          <article className="max-w-2xl mx-auto">
            {/* Article Masthead */}
            <header className="mb-8 pb-6 border-b border-neutral-100 dark:border-neutral-900 space-y-3">
              <h1
                id="reader-modal-title"
                className="text-2xl sm:text-3xl font-semibold tracking-tight text-neutral-900 dark:text-neutral-100 font-sans"
              >
                {bookmark.title}
              </h1>

              <div className="flex flex-wrap items-center gap-3 text-xs text-neutral-500 font-sans">
                {bookmark.author && (
                  <span>By <strong className="font-medium text-neutral-700 dark:text-neutral-300">{bookmark.author}</strong></span>
                )}
                <span>Published on {bookmark.domain}</span>
                <span>·</span>
                <span className="font-mono text-[11px]">{new Date(bookmark.created_at).toLocaleDateString()}</span>
              </div>

              {bookmark.description && (
                <p className="text-sm text-neutral-600 dark:text-neutral-400 italic pt-1">
                  {bookmark.description}
                </p>
              )}
            </header>

            {/* Article Body */}
            {paragraphs.length > 0 ? (
              <div className={`space-y-4 text-neutral-800 dark:text-neutral-200 font-sans ${textSizeClass}`}>
                {paragraphs.map((p, idx) => (
                  <p key={idx} className="leading-relaxed">
                    {p}
                  </p>
                ))}
              </div>
            ) : (
              <div className="py-12 text-center space-y-4">
                <p className="text-sm text-neutral-500">
                  Full article reader text could not be extracted for this page.
                </p>
                <a
                  href={bookmark.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1.5 px-4 py-2 rounded bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-neutral-100 dark:hover:bg-neutral-200 dark:text-neutral-900 text-xs font-medium transition cursor-pointer"
                >
                  <span>Read original article on {bookmark.domain}</span>
                  <ExternalLink className="w-3.5 h-3.5" />
                </a>
              </div>
            )}
          </article>
        </div>
      </div>
    </div>
  );
};
