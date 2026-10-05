import React, { useState } from 'react';
import { Plus, Search, Archive, ArchiveRestore, Sparkles, BookOpen } from 'lucide-react';
import { useNotes, useNoteTags, useCreateNote, useUpdateNote, useDeleteNote } from './api';
import { NoteCard } from './NoteCard';
import { NoteEditorModal } from './NoteEditorModal';
import { KnowledgeSearchPanel } from './KnowledgeSearchPanel';
import { TagFilterChips } from './TagFilterChips';
import { ConfirmModal } from '../../components/ConfirmModal';
import type { Note, SearchResultItem } from './types';

export const KnowledgeMainView: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'active' | 'archived'>('active');
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [showHybridSearch, setShowHybridSearch] = useState(false);

  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editingNote, setEditingNote] = useState<Note | null>(null);
  const [noteToDelete, setNoteToDelete] = useState<Note | null>(null);

  const { data: tags = [] } = useNoteTags();
  const { data: notes = [], isLoading } = useNotes({
    archived: activeTab === 'archived',
    tag: selectedTag,
    search: searchQuery,
  });

  const createMutation = useCreateNote();
  const updateMutation = useUpdateNote();
  const deleteMutation = useDeleteNote();

  const handleOpenCreate = () => {
    setEditingNote(null);
    setIsEditorOpen(true);
  };

  const handleOpenEdit = (note: Note) => {
    setEditingNote(note);
    setIsEditorOpen(true);
  };

  const handleSaveNote = async (data: {
    title: string;
    content: string;
    format: string;
    tags: string[];
  }) => {
    if (editingNote) {
      await updateMutation.mutateAsync({
        id: editingNote.id,
        input: {
          title: data.title,
          content: data.content,
          format: data.format,
          tags: data.tags,
        },
      });
    } else {
      await createMutation.mutateAsync({
        title: data.title,
        content: data.content,
        format: data.format,
        tags: data.tags,
      });
    }
  };

  const handleToggleArchive = async (note: Note) => {
    await updateMutation.mutateAsync({
      id: note.id,
      input: {
        is_archived: !note.is_archived,
      },
    });
  };

  const handleDeleteConfirm = async () => {
    if (noteToDelete) {
      await deleteMutation.mutateAsync(noteToDelete.id);
      setNoteToDelete(null);
    }
  };

  const handleSelectSearchResult = (item: SearchResultItem) => {
    if (item.item_type === 'note') {
      const match = notes.find((n) => n.id === item.item_id);
      if (match) {
        handleOpenEdit(match);
      }
    }
  };

  return (
    <div className="space-y-6 max-w-7xl mx-auto w-full">
      {/* Top Action & Navigation Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        {/* Tabs: Active vs Archived */}
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-800 p-1 rounded-lg shrink-0">
          <button
            type="button"
            onClick={() => setActiveTab('active')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
              activeTab === 'active'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
            }`}
          >
            Active Notes
          </button>
          <button
            type="button"
            onClick={() => setActiveTab('archived')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
              activeTab === 'archived'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
            }`}
          >
            Archive
          </button>
        </div>

        {/* Action Buttons: Toggle Search & New Note */}
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setShowHybridSearch((prev) => !prev)}
            className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors ${
              showHybridSearch
                ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 border-neutral-900 dark:border-neutral-100'
                : 'bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-800'
            }`}
          >
            <Sparkles className="w-3.5 h-3.5" />
            <span>AI Search</span>
          </button>

          <button
            type="button"
            data-testid="create-note-button"
            onClick={handleOpenCreate}
            className="flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-medium rounded-lg bg-neutral-900 text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors shadow-xs"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>New Note</span>
          </button>
        </div>
      </div>

      {/* Semantic Hybrid Search Panel */}
      {showHybridSearch && (
        <KnowledgeSearchPanel onSelectResult={handleSelectSearchResult} />
      )}

      {/* Search Filter & Tag Selector Bar */}
      <div className="space-y-2">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-neutral-400" />
          <input
            type="text"
            data-testid="notes-search-input"
            placeholder="Filter notes by title or text..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-9 pr-4 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-lg text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
          />
        </div>

        <TagFilterChips
          tags={tags}
          selectedTag={selectedTag}
          onSelectTag={setSelectedTag}
        />
      </div>

      {/* Notes Grid */}
      {isLoading ? (
        <div className="py-16 text-center text-xs text-neutral-400 font-mono">
          Loading notes...
        </div>
      ) : notes.length === 0 ? (
        <div className="py-16 text-center rounded-xl border border-dashed border-neutral-300 dark:border-neutral-800 p-8 space-y-3">
          <BookOpen className="w-8 h-8 text-neutral-400 mx-auto" />
          <div className="space-y-1">
            <p className="text-sm font-medium text-neutral-800 dark:text-neutral-200">
              {activeTab === 'archived' ? 'No archived notes' : 'No notes yet'}
            </p>
            <p className="text-xs text-neutral-500">
              {selectedTag
                ? `No notes found tagged #${selectedTag}`
                : activeTab === 'archived'
                ? 'Notes you archive will show up here.'
                : 'Capture ideas, snippets, and research into your second brain.'}
            </p>
          </div>
          {activeTab === 'active' && !selectedTag && (
            <button
              type="button"
              onClick={handleOpenCreate}
              className="inline-flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-medium rounded-lg bg-neutral-900 text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>Create first note</span>
            </button>
          )}
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {notes.map((note) => (
            <NoteCard
              key={note.id}
              note={note}
              onEdit={handleOpenEdit}
              onDelete={setNoteToDelete}
              onToggleArchive={handleToggleArchive}
              onSelectTag={setSelectedTag}
            />
          ))}
        </div>
      )}

      {/* Editor Modal */}
      <NoteEditorModal
        isOpen={isEditorOpen}
        onClose={() => setIsEditorOpen(false)}
        onSave={handleSaveNote}
        initialNote={editingNote}
        isSaving={createMutation.isPending || updateMutation.isPending}
      />

      {/* Delete Confirmation Modal */}
      <ConfirmModal
        isOpen={noteToDelete !== null}
        onClose={() => setNoteToDelete(null)}
        onConfirm={handleDeleteConfirm}
        title="Delete Note"
        message={
          <>
            Are you sure you want to permanently delete{' '}
            <span className="font-semibold text-neutral-900 dark:text-neutral-100">
              "{noteToDelete?.title}"
            </span>
            ? This cannot be undone.
          </>
        }
        confirmLabel="Delete Note"
        isDestructive
        isLoading={deleteMutation.isPending}
      />
    </div>
  );
};
