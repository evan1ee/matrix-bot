import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi, fetchApiResponse } from '../../core/apiClient';
import type {
  Note,
  NoteSummary,
  CreateNoteInput,
  UpdateNoteInput,
  SearchResultItem,
  KnowledgeSearchResponse,
} from './types';

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

  return useQuery<NoteSummary[]>({
    queryKey: ['notes', queryString],
    queryFn: () => fetchApi<NoteSummary[]>(endpoint),
  });
}

/**
 * One note with its body. Lists carry only an excerpt, so opening a note
 * fetches it; `gcTime: 0` makes every open start from the server's copy
 * rather than a cached one.
 */
export function useNote(id: number | null) {
  return useQuery<Note>({
    queryKey: ['notes', 'document', id],
    queryFn: () => fetchApi<Note>(`/notes/${id}`),
    enabled: id !== null,
    gcTime: 0,
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
  return useQuery<KnowledgeSearchResponse>({
    queryKey: ['knowledge-search', trimmed, filterTypes, limit],
    queryFn: async () => {
      const { data, headers } = await fetchApiResponse<SearchResultItem[]>('/knowledge/search', {
        method: 'POST',
        body: JSON.stringify({
          query: trimmed,
          filter_types: filterTypes && filterTypes.length > 0 ? filterTypes : null,
          limit,
        }),
      });
      return {
        items: data,
        // The server says so in a header because the body is a bare array.
        mode: headers.get('x-search-mode') === 'lexical' ? 'lexical' : 'hybrid',
      };
    },
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
