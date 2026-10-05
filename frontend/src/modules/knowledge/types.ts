import type { components } from '../../types/api';

export type Note = components['schemas']['Note'];
export type CreateNoteInput = components['schemas']['CreateNoteInput'];
export type UpdateNoteInput = components['schemas']['UpdateNoteInput'];
export type SearchResultItem = components['schemas']['SearchResultItem'];
export type KnowledgeSearchRequest = components['schemas']['KnowledgeSearchRequest'];

export interface NoteFilterState {
  tab: 'active' | 'archived';
  selectedTag: string | null;
  searchQuery: string;
}
