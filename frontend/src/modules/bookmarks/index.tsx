import React, { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Bookmark as BookmarkIcon,
  Plus,
  Search,
  BookOpen,
  Star,
  Archive,
  Tag,
  Globe,
  X,
  Sparkles,
} from 'lucide-react';

import type { FrontendModule, SystemEvent } from '../../core/types';
import { triggerAiPrompt } from '../ai';
import type { Bookmark, BookmarkStats, CreateBookmarkInput, BookmarkFilterTab } from './types';
import { BookmarkCard } from './BookmarkCard';
import { ReaderModal } from './ReaderModal';
import { AddBookmarkModal } from './AddBookmarkModal';
import { ConfirmModal } from '../../components/ConfirmModal';

export const BookmarksView: React.FC = () => {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<BookmarkFilterTab>('unread');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [readingBookmark, setReadingBookmark] = useState<Bookmark | null>(null);
  const [bookmarkToDelete, setBookmarkToDelete] = useState<Bookmark | null>(null);

  // Derive query parameters
  const queryParams = useMemo(() => {
    const params = new URLSearchParams();
    if (activeTab === 'unread') {
      params.set('archived', 'false');
    } else if (activeTab === 'favorites') {
      params.set('favorite', 'true');
    } else if (activeTab === 'archive') {
      params.set('archived', 'true');
    }
    if (selectedTag) {
      params.set('tag', selectedTag);
    }
    if (searchQuery.trim()) {
      params.set('search', searchQuery.trim());
    }
    return params.toString();
  }, [activeTab, selectedTag, searchQuery]);

  // Query: Bookmarks list
  const { data: bookmarks = [], isLoading } = useQuery<Bookmark[]>({
    queryKey: ['bookmarks', queryParams],
    queryFn: async () => {
      const url = `/api/v1/bookmarks${queryParams ? `?${queryParams}` : ''}`;
      const res = await fetch(url, { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load bookmarks');
      return res.json();
    },
  });

  // Query: Stats
  const { data: stats } = useQuery<BookmarkStats>({
    queryKey: ['bookmarks-stats'],
    queryFn: async () => {
      const res = await fetch('/api/v1/bookmarks/stats', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load bookmark statistics');
      return res.json();
    },
  });

  // Mutation: Create
  const createMutation = useMutation({
    mutationFn: async (payload: CreateBookmarkInput) => {
      const res = await fetch('/api/v1/bookmarks', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(payload),
      });
      if (!res.ok) {
        let msg = `Failed to save bookmark (${res.status})`;
        try {
          const data = await res.json();
          if (data.error) msg = data.error;
        } catch {
          // Response body is not JSON
        }
        throw new Error(msg);
      }
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['bookmarks'] });
      queryClient.invalidateQueries({ queryKey: ['bookmarks-stats'] });
    },
  });

  // Mutation: Toggle Favorite
  const toggleFavoriteMutation = useMutation({
    mutationFn: async (id: number) => {
      const res = await fetch(`/api/v1/bookmarks/${id}/toggle-favorite`, {
        method: 'POST',
        credentials: 'include',
      });
      if (!res.ok) throw new Error('Failed to toggle favorite');
      return res.json();
    },
    onSuccess: (updated: Bookmark) => {
      queryClient.invalidateQueries({ queryKey: ['bookmarks'] });
      queryClient.invalidateQueries({ queryKey: ['bookmarks-stats'] });
      if (readingBookmark && readingBookmark.id === updated.id) {
        setReadingBookmark(updated);
      }
    },
  });

  // Mutation: Toggle Archive
  const toggleArchiveMutation = useMutation({
    mutationFn: async (id: number) => {
      const res = await fetch(`/api/v1/bookmarks/${id}/toggle-archive`, {
        method: 'POST',
        credentials: 'include',
      });
      if (!res.ok) throw new Error('Failed to toggle archive');
      return res.json();
    },
    onSuccess: (updated: Bookmark) => {
      queryClient.invalidateQueries({ queryKey: ['bookmarks'] });
      queryClient.invalidateQueries({ queryKey: ['bookmarks-stats'] });
      if (readingBookmark && readingBookmark.id === updated.id) {
        setReadingBookmark(updated);
      }
    },
  });

  // Mutation: Delete
  const deleteMutation = useMutation({
    mutationFn: async (id: number) => {
      const res = await fetch(`/api/v1/bookmarks/${id}`, {
        method: 'DELETE',
        credentials: 'include',
      });
      if (!res.ok) throw new Error('Failed to delete bookmark');
    },
    onSuccess: (_, deletedId) => {
      queryClient.invalidateQueries({ queryKey: ['bookmarks'] });
      queryClient.invalidateQueries({ queryKey: ['bookmarks-stats'] });
      if (readingBookmark && readingBookmark.id === deletedId) {
        setReadingBookmark(null);
      }
    },
  });

  return (
    <div className="space-y-6">
      {/* Masthead */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-neutral-200 dark:border-neutral-800">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-neutral-900 dark:text-neutral-100 font-sans">
            Bookmarks & Reading List
          </h1>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1 font-sans">
            Clean reader view, metadata extraction, and tag categorization.
          </p>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            onClick={() =>
              triggerAiPrompt(
                'Review my unread and favorite bookmarks. Recommend which article or guide I should read next and summarize why it is relevant.'
              )
            }
            className="h-9 inline-flex items-center gap-1.5 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 text-xs font-medium hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
            title="Ask AI for reading suggestions"
          >
            <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
            <span>AI Suggestion</span>
          </button>

          <button
            type="button"
            data-testid="save-bookmark-btn"
            onClick={() => setIsAddModalOpen(true)}
            className="h-9 inline-flex items-center gap-1.5 px-3.5 rounded-md bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-neutral-100 dark:hover:bg-neutral-200 dark:text-neutral-900 text-xs font-medium transition shadow-xs cursor-pointer shrink-0"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Save Link</span>
          </button>
        </div>
      </div>

      {/* Sourced Stat Strip (Vercel-style) */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <div className="p-3.5 rounded-lg bg-neutral-50 dark:bg-neutral-900/40 border border-neutral-200/80 dark:border-neutral-800/80">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">Unread</p>
          <p className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 mt-1 font-sans">
            {stats?.unread_count ?? 0}
          </p>
        </div>
        <div className="p-3.5 rounded-lg bg-neutral-50 dark:bg-neutral-900/40 border border-neutral-200/80 dark:border-neutral-800/80">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">Favorites</p>
          <p className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 mt-1 font-sans">
            {stats?.favorite_count ?? 0}
          </p>
        </div>
        <div className="p-3.5 rounded-lg bg-neutral-50 dark:bg-neutral-900/40 border border-neutral-200/80 dark:border-neutral-800/80">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">Total Saved</p>
          <p className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 mt-1 font-sans">
            {stats?.total_count ?? 0}
          </p>
        </div>
        <div className="p-3.5 rounded-lg bg-neutral-50 dark:bg-neutral-900/40 border border-neutral-200/80 dark:border-neutral-800/80">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">Archived</p>
          <p className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 mt-1 font-sans">
            {stats?.archived_count ?? 0}
          </p>
        </div>
      </div>

      {/* Navigation Tabs and Search */}
      <div className="space-y-3">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          {/* Tabs */}
          <div className="flex items-center gap-1 border-b border-neutral-200 dark:border-neutral-800 pb-px">
            <button
              type="button"
              data-testid="tab-unread"
              onClick={() => setActiveTab('unread')}
              className={`px-3 py-1.5 text-xs font-medium rounded-t transition cursor-pointer ${
                activeTab === 'unread'
                  ? 'text-neutral-900 dark:text-neutral-100 border-b-2 border-neutral-900 dark:border-neutral-100 font-semibold'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Unread ({stats?.unread_count ?? 0})
            </button>
            <button
              type="button"
              data-testid="tab-favorites"
              onClick={() => setActiveTab('favorites')}
              className={`px-3 py-1.5 text-xs font-medium rounded-t transition cursor-pointer ${
                activeTab === 'favorites'
                  ? 'text-neutral-900 dark:text-neutral-100 border-b-2 border-neutral-900 dark:border-neutral-100 font-semibold'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Favorites ({stats?.favorite_count ?? 0})
            </button>
            <button
              type="button"
              data-testid="tab-archive"
              onClick={() => setActiveTab('archive')}
              className={`px-3 py-1.5 text-xs font-medium rounded-t transition cursor-pointer ${
                activeTab === 'archive'
                  ? 'text-neutral-900 dark:text-neutral-100 border-b-2 border-neutral-900 dark:border-neutral-100 font-semibold'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Archive ({stats?.archived_count ?? 0})
            </button>
            <button
              type="button"
              data-testid="tab-all"
              onClick={() => setActiveTab('all')}
              className={`px-3 py-1.5 text-xs font-medium rounded-t transition cursor-pointer ${
                activeTab === 'all'
                  ? 'text-neutral-900 dark:text-neutral-100 border-b-2 border-neutral-900 dark:border-neutral-100 font-semibold'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              All ({stats?.total_count ?? 0})
            </button>
          </div>

          {/* Search box */}
          <div className="relative w-full sm:w-64">
            <Search className="w-3.5 h-3.5 text-neutral-400 absolute left-2.5 top-1/2 -translate-y-1/2" />
            <input
              type="text"
              placeholder="Search title, URL, or tags..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full h-9 pl-8 pr-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>
        </div>

        {/* Tag Filters if tags exist */}
        {stats?.tags && stats.tags.length > 0 && (
          <div data-testid="tag-filters" className="flex items-center gap-1.5 flex-wrap pt-1">
            <span className="text-[10px] font-mono text-neutral-400 uppercase tracking-wider mr-1">
              Tags:
            </span>
            {selectedTag && (
              <button
                type="button"
                onClick={() => setSelectedTag(null)}
                className="inline-flex items-center gap-1 text-[10px] font-mono px-2 py-0.5 rounded bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 cursor-pointer"
              >
                <span>#{selectedTag}</span>
                <X className="w-3 h-3" />
              </button>
            )}
            {stats.tags.map((tag) => {
              if (tag === selectedTag) return null;
              return (
                <button
                  key={tag}
                  type="button"
                  onClick={() => setSelectedTag(tag)}
                  className="text-[10px] font-mono px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800/70 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700 transition cursor-pointer"
                >
                  #{tag}
                </button>
              );
            })}
          </div>
        )}
      </div>

      {/* Main Grid */}
      {isLoading ? (
        <div className="py-16 text-center text-xs text-neutral-500 font-mono">
          Loading bookmarks...
        </div>
      ) : bookmarks.length === 0 ? (
        <div className="py-16 text-center rounded-lg border border-dashed border-neutral-200 dark:border-neutral-800 space-y-3">
          <BookmarkIcon className="w-8 h-8 mx-auto text-neutral-300 dark:text-neutral-700" />
          <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100 font-sans">
            No bookmarks found
          </p>
          <p className="text-xs text-neutral-500 font-sans max-w-sm mx-auto">
            {searchQuery || selectedTag
              ? 'Try changing your search query or tag filter.'
              : 'Save articles or web pages using the button above or with the bot command !save <url>.'}
          </p>
          {!searchQuery && !selectedTag && (
            <button
              type="button"
              onClick={() => setIsAddModalOpen(true)}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-neutral-100 dark:hover:bg-neutral-200 dark:text-neutral-900 text-xs font-medium transition cursor-pointer"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>Save First Bookmark</span>
            </button>
          )}
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {bookmarks.map((bookmark) => (
            <BookmarkCard
              key={bookmark.id}
              bookmark={bookmark}
              onOpenReader={(b) => setReadingBookmark(b)}
              onToggleFavorite={(id) => toggleFavoriteMutation.mutate(id)}
              onToggleArchive={(id) => toggleArchiveMutation.mutate(id)}
              onDelete={() => setBookmarkToDelete(bookmark)}
              onSelectTag={(t) => setSelectedTag(t)}
            />
          ))}
        </div>
      )}

      {/* Reader Modal */}
      <ReaderModal
        bookmark={readingBookmark}
        isOpen={!!readingBookmark}
        onClose={() => setReadingBookmark(null)}
        onToggleFavorite={(id) => toggleFavoriteMutation.mutate(id)}
        onToggleArchive={(id) => toggleArchiveMutation.mutate(id)}
      />

      {/* Add Bookmark Modal */}
      <AddBookmarkModal
        isOpen={isAddModalOpen}
        onClose={() => setIsAddModalOpen(false)}
        onSave={async (payload) => {
          await createMutation.mutateAsync(payload);
        }}
      />

      {/* Delete Confirmation Modal */}
      <ConfirmModal
        isOpen={!!bookmarkToDelete}
        title="Delete Bookmark"
        message={
          <>
            Are you sure you want to delete{' '}
            <span className="font-semibold text-neutral-900 dark:text-neutral-100">
              "{bookmarkToDelete?.title}"
            </span>
            ?
          </>
        }
        confirmLabel="Delete bookmark"
        isDestructive
        isLoading={deleteMutation.isPending}
        onConfirm={async () => {
          if (bookmarkToDelete) {
            await deleteMutation.mutateAsync(bookmarkToDelete.id);
            setBookmarkToDelete(null);
          }
        }}
        onClose={() => setBookmarkToDelete(null)}
      />
    </div>
  );
};

export const bookmarksModule: FrontendModule = {
  id: 'bookmarks',
  name: 'Bookmarks',
  description: 'Pocket/Instapaper-style read-it-later article library',
  iconName: 'Bookmark',
  navTab: {
    label: 'Bookmarks',
  },
  MainView: BookmarksView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (
      event.type === 'BookmarkCreated' ||
      event.type === 'BookmarkUpdated' ||
      event.type === 'BookmarkDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['bookmarks'] });
      queryClient.invalidateQueries({ queryKey: ['bookmarks-stats'] });
    }
  },
};
