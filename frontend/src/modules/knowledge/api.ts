import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi, fetchApiBlob, fetchApiResponse } from '../../core/apiClient';
import { filenameFromDisposition, saveBlob } from '../../core/download';
import { useSensitiveAction } from '../../core/security';
import type {
  Note,
  NoteSummary,
  CreateNoteInput,
  UpdateNoteInput,
  SearchResultItem,
  KnowledgeSearchResponse,
  ImportReport,
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

/** The server allows archive requests up to 180 s; the client must not give up first. */
const ARCHIVE_TIMEOUT_MS = 200_000;

/**
 * Downloads every note as a zip of Markdown files. Needs a recently verified
 * session, so a stale one is asked for a passkey and the download retried.
 */
export function useExportNotes() {
  return useSensitiveAction<void, void>(async () => {
    const { blob, headers } = await fetchApiBlob('/notes/export', { timeoutMs: ARCHIVE_TIMEOUT_MS });
    saveBlob(blob, filenameFromDisposition(headers.get('content-disposition')) ?? 'notes.zip');
  });
}

/** Uploads a zip of Markdown files; same step-up handling as the export. */
export function useImportNotes() {
  return useSensitiveAction<File, ImportReport>(
    (file) => {
      const form = new FormData();
      form.append('file', file);
      return fetchApi<ImportReport>('/notes/import', {
        method: 'POST',
        body: form,
        timeoutMs: ARCHIVE_TIMEOUT_MS,
      });
    },
    [['notes'], ['notes-tags']]
  );
}
