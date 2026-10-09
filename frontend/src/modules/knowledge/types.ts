import type { components } from '../../types/api';

/** A note as listed: metadata and an excerpt, never the body. */
export type NoteSummary = components['schemas']['NoteSummary'];
/** A single note: the summary fields plus the Markdown body. */
export type Note = components['schemas']['Note'];
export type CreateNoteInput = components['schemas']['CreateNoteInput'];
export type UpdateNoteInput = components['schemas']['UpdateNoteInput'];
export type SearchResultItem = components['schemas']['SearchResultItem'];
export type KnowledgeSearchRequest = components['schemas']['KnowledgeSearchRequest'];

/** Mirrors `domain::knowledge::note::NoteType`. */
export const NOTE_TYPES = [
  { value: 'note', label: 'Note' },
  { value: 'idea', label: 'Idea' },
  { value: 'fact', label: 'Fact' },
  { value: 'observation', label: 'Observation' },
  { value: 'decision', label: 'Decision' },
] as const;

export type NoteTypeValue = (typeof NOTE_TYPES)[number]['value'];

/** `hybrid`, or `lexical` when the server could only rank by keywords. */
export type SearchMode = 'hybrid' | 'lexical';

export interface KnowledgeSearchResponse {
  items: SearchResultItem[];
  mode: SearchMode;
}

export interface NoteFilterState {
  tab: 'active' | 'archived';
  selectedTag: string | null;
  searchQuery: string;
}
