import React from 'react';
import { Star, Archive, ArchiveRestore, BookOpen, ExternalLink, Trash2, Globe } from 'lucide-react';
import type { Bookmark } from './types';

interface BookmarkCardProps {
  bookmark: Bookmark;
  onOpenReader: (bookmark: Bookmark) => void;
  onToggleFavorite: (id: number) => void;
  onToggleArchive: (id: number) => void;
  onDelete: (id: number) => void;
  onSelectTag?: (tag: string) => void;
}

export const BookmarkCard: React.FC<BookmarkCardProps> = ({
  bookmark,
  onOpenReader,
  onToggleFavorite,
  onToggleArchive,
  onDelete,
  onSelectTag,
}) => {
  const tagsList = bookmark.tags
    ? bookmark.tags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean)
    : [];

  return (
    <article
      data-testid={`bookmark-card-${bookmark.id}`}
      className="group relative flex flex-col justify-between rounded-lg bg-white dark:bg-neutral-900/60 border border-neutral-200 dark:border-neutral-800/80 hover:border-neutral-400 dark:hover:border-neutral-700 transition p-4 space-y-3"
    >
      <div className="space-y-2.5">
        {/* Top Meta & Star/Archive */}
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5 min-w-0">
            <Globe className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
            <span className="text-[11px] font-mono uppercase tracking-wider text-neutral-500 truncate">
              {bookmark.domain}
            </span>
            {bookmark.reading_time_minutes > 0 && (
              <>
                <span className="text-neutral-300 dark:text-neutral-700">·</span>
                <span className="text-[11px] font-mono text-neutral-500 shrink-0">
                  {bookmark.reading_time_minutes}m
                </span>
              </>
            )}
          </div>

          <div className="flex items-center gap-1 shrink-0">
            {/* Favorite toggle */}
            <button
              type="button"
              onClick={() => onToggleFavorite(bookmark.id)}
              aria-label={bookmark.is_favorite ? 'Unstar bookmark' : 'Star bookmark'}
              className={`p-1 rounded transition cursor-pointer ${
                bookmark.is_favorite
                  ? 'text-amber-500 hover:text-amber-600'
                  : 'text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300'
              }`}
            >
              <Star
                className={`w-3.5 h-3.5 ${bookmark.is_favorite ? 'fill-amber-500' : ''}`}
              />
            </button>

            {/* Archive toggle */}
            <button
              type="button"
              onClick={() => onToggleArchive(bookmark.id)}
              aria-label={bookmark.is_archived ? 'Unarchive bookmark' : 'Archive bookmark'}
              className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300 transition cursor-pointer"
            >
              {bookmark.is_archived ? (
                <ArchiveRestore className="w-3.5 h-3.5" />
              ) : (
                <Archive className="w-3.5 h-3.5" />
              )}
            </button>
          </div>
        </div>

        {/* Thumbnail if available */}
        {bookmark.image_url && (
          <div className="w-full h-32 overflow-hidden rounded bg-neutral-100 dark:bg-neutral-800">
            <img
              src={bookmark.image_url}
              alt=""
              loading="lazy"
              onError={(e) => {
                // Hide broken images gracefully
                (e.target as HTMLElement).style.display = 'none';
              }}
              className="w-full h-full object-cover transition-opacity duration-150"
            />
          </div>
        )}

        {/* Title */}
        <h3
          onClick={() => onOpenReader(bookmark)}
          className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 hover:underline cursor-pointer line-clamp-2 leading-snug font-sans"
        >
          {bookmark.title}
        </h3>

        {/* Description snippet */}
        {bookmark.description && (
          <p className="text-xs text-neutral-500 dark:text-neutral-400 line-clamp-2 font-sans leading-relaxed">
            {bookmark.description}
          </p>
        )}
      </div>

      {/* Bottom Row: Tags & Actions */}
      <div className="pt-2 border-t border-neutral-100 dark:border-neutral-800/60 flex items-center justify-between gap-2">
        {/* Tags */}
        <div className="flex items-center gap-1.5 flex-wrap min-w-0">
          {tagsList.map((tag) => (
            <button
              key={tag}
              type="button"
              onClick={() => onSelectTag?.(tag)}
              className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800/80 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700 transition cursor-pointer"
            >
              #{tag}
            </button>
          ))}
        </div>

        {/* Action icons */}
        <div className="flex items-center gap-1 shrink-0">
          <button
            type="button"
            onClick={() => onOpenReader(bookmark)}
            title="Open in Reader View"
            className="p-1 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <BookOpen className="w-3.5 h-3.5" />
          </button>
          <a
            href={bookmark.url}
            target="_blank"
            rel="noopener noreferrer"
            title="Open original website"
            className="p-1 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <ExternalLink className="w-3.5 h-3.5" />
          </a>
          <button
            type="button"
            onClick={() => {
              if (window.confirm(`Delete bookmark "${bookmark.title}"?`)) {
                onDelete(bookmark.id);
              }
            }}
            title="Delete bookmark"
            className="p-1 rounded text-neutral-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/30 transition cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </article>
  );
};
