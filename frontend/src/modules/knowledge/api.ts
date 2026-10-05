import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import type { Note, CreateNoteInput, UpdateNoteInput, SearchResultItem } from './types';

export function useNotes(filter?: { tag?: string | null; archived?: boolean; search?: string }) {
  const queryParams = new URLSearchParams();
  if (filter?.archived !== undefined) {
    queryParams.set('archived', String(filter.archived));
  }
  if (filter?.tag) {
    queryParams.set('tag', filter.tag);
  }
  if (filter?.search?.trim()) {
    queryParams.set('search', filter.search.trim());
  }

  const queryString = queryParams.toString();
  const endpoint = `/notes${queryString ? `?${queryString}` : ''}`;

  return useQuery<Note[]>({
    queryKey: ['notes', queryString],
    queryFn: () => fetchApi<Note[]>(endpoint),
  });
}

export function useNoteTags() {
  return useQuery<string[]>({
    queryKey: ['notes-tags'],
    queryFn: () => fetchApi<string[]>('/notes/tags'),
  });
}

export function useKnowledgeSearch(query: string, filterTypes?: string[], limit: number = 8) {
  const trimmed = query.trim();
  return useQuery<SearchResultItem[]>({
    queryKey: ['knowledge-search', trimmed, filterTypes, limit],
    queryFn: () =>
      fetchApi<SearchResultItem[]>('/knowledge/search', {
        method: 'POST',
        body: JSON.stringify({
          query: trimmed,
          filter_types: filterTypes && filterTypes.length > 0 ? filterTypes : null,
          limit,
        }),
      }),
    enabled: trimmed.length > 0,
  });
}

export function useCreateNote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateNoteInput) =>
      fetchApi<Note>('/notes', {
        method: 'POST',
        body: JSON.stringify(input),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['notes'] });
      queryClient.invalidateQueries({ queryKey: ['notes-tags'] });
    },
  });
}

export function useUpdateNote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id: number; input: UpdateNoteInput }) =>
      fetchApi<Note>(`/notes/${id}`, {
        method: 'PUT',
        body: JSON.stringify(input),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['notes'] });
      queryClient.invalidateQueries({ queryKey: ['notes-tags'] });
    },
  });
}

export function useDeleteNote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) =>
      fetchApi<void>(`/notes/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['notes'] });
      queryClient.invalidateQueries({ queryKey: ['notes-tags'] });
    },
  });
}
