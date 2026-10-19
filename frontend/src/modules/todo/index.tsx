import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import { Sparkles } from 'lucide-react';
import { triggerAiPrompt } from '../ai';
import { useSettings, zonedInputToUtcIso } from '../../core/settings';
import { TodoItem, TodoListResponse, TodoTagsResponse, JoinedRoomInfo, TodoPatch } from './types';
import { TodoRow } from './components/TodoRow';
import { CreateTodoCard } from './components/CreateTodoCard';

export type { TodoItem, TodoListResponse, JoinedRoomInfo, TodoPatch };

export const TodoMainView: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const { timezone } = useSettings();
  const queryClient = useQueryClient();
  const textInputRef = React.useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const [priority, setPriority] = useState('medium');
  const [dueAt, setDueAt] = useState('');
  const [leadMinutes, setLeadMinutes] = useState('');
  const [remindRoomId, setRemindRoomId] = useState('');
  const [filter, setFilter] = useState<'all' | 'active' | 'completed'>('all');
  const [tags, setTags] = useState<string[]>([]);
  const [recurrence, setRecurrence] = useState('none');
  const [tagFilter, setTagFilter] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const completedQuery =
    filter === 'active' ? '&completed=false' : filter === 'completed' ? '&completed=true' : '';
  const tagQuery = tagFilter ? `&tag=${encodeURIComponent(tagFilter)}` : '';

  const { data, isLoading } = useQuery<TodoListResponse>({
    queryKey: ['todos', filter, tagFilter],
    queryFn: () => fetchApi<TodoListResponse>(`/todos?limit=100${completedQuery}${tagQuery}`),
  });

  // Not under the ['todos'] prefix: the optimistic toggle rewrites every
  // ['todos'] cache entry as a list, which this one is not.
  const { data: tagData } = useQuery<TodoTagsResponse>({
    queryKey: ['todo-tags'],
    queryFn: () => fetchApi<TodoTagsResponse>('/todos/tags'),
  });
  const refreshTodos = () => {
    queryClient.invalidateQueries({ queryKey: ['todos'] });
    queryClient.invalidateQueries({ queryKey: ['todo-tags'] });
    queryClient.invalidateQueries({ queryKey: ['todo-detail'] });
  };

  // Rooms the bot and this account are both in. Local (@web-*) accounts share
  // no room with the bot, so this comes back empty for them by design.
  const { data: rooms } = useQuery<JoinedRoomInfo[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  React.useEffect(() => {
    if (autoFocus) textInputRef.current?.focus();
  }, [autoFocus]);

  const wantsReminder = leadMinutes !== '';

  const createMutation = useMutation({
    mutationFn: (newTodo: {
      room_id: string;
      text: string;
      priority: string;
      due_at: string | null;
      remind_before_minutes: number | null;
      remind_room_id: string | null;
      tags: string[];
      recurrence: string;
    }) =>
      fetchApi<TodoItem>('/todos', {
        method: 'POST',
        body: JSON.stringify(newTodo),
      }),
    onSuccess: () => {
      setText('');
      setDueAt('');
      setLeadMinutes('');
      setTags([]);
      setRecurrence('none');
      setErrorMsg(null);
      refreshTodos();
      // Creating a task with a lead time also creates a reminder row.
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
    onError: (err: Error) => {
      setErrorMsg(err.message);
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, completed, priority }: { id: number; completed?: boolean; priority?: string }) =>
      fetchApi<TodoItem>(`/todos/${id}`, {
        method: 'PATCH',
        body: JSON.stringify({ completed, priority }),
      }),
    onMutate: async ({ id, completed, priority }) => {
      await queryClient.cancelQueries({ queryKey: ['todos'] });
      const previousTodos = queryClient.getQueriesData<TodoListResponse>({ queryKey: ['todos'] });
      queryClient.setQueriesData<TodoListResponse>({ queryKey: ['todos'] }, (old) => {
        if (!old) return old;
        return {
          ...old,
          items: old.items.map((item) => {
            if (item.id !== id) return item;
            return {
              ...item,
              ...(completed !== undefined ? { completed } : {}),
              ...(priority !== undefined ? { priority } : {}),
            };
          }),
        };
      });
      return { previousTodos };
    },
    onError: (_err, _variables, context) => {
      if (context?.previousTodos) {
        context.previousTodos.forEach(([key, oldData]) => {
          queryClient.setQueryData(key, oldData);
        });
      }
    },
    onSettled: refreshTodos,
  });

  // Full-field edit. Unlike `updateMutation`, this always states `due_at` and
  // `remind_before_minutes` explicitly — `null` clears them — because the edit
  // form owns those values outright.
  const editMutation = useMutation({
    mutationFn: ({ id, ...patch }: TodoPatch & { id: number }) =>
      fetchApi<TodoItem>(`/todos/${id}`, {
        method: 'PATCH',
        body: JSON.stringify(patch),
      }),
    onSuccess: () => {
      refreshTodos();
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/todos/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      refreshTodos();
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
  });

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    if (text.length > 500) {
      setErrorMsg('Task text exceeds 500 character limit.');
      return;
    }

    const dueIso = zonedInputToUtcIso(dueAt, timezone);
    if (wantsReminder && !dueIso) {
      setErrorMsg('Pick a due date for the reminder to count back from.');
      return;
    }
    if (wantsReminder && !remindRoomId) {
      setErrorMsg('Choose a Matrix room for the reminder to be sent to.');
      return;
    }

    createMutation.mutate({
      room_id: 'web',
      text: text.trim(),
      priority,
      due_at: dueIso,
      remind_before_minutes: wantsReminder ? Number(leadMinutes) : null,
      remind_room_id: wantsReminder ? remindRoomId : null,
      tags,
      // A repeat counts forward from the due date, so it is moot without one.
      recurrence: dueIso ? recurrence : 'none',
    });
  };

  const filteredItems = data?.items || [];
  // Keep the active filter selectable even once its last active todo is done
  // (the server only counts active todos).
  const tagOptions =
    tagFilter && !tagData?.tags.some((t) => t.tag === tagFilter)
      ? [...(tagData?.tags ?? []), { tag: tagFilter, count: 0 }]
      : (tagData?.tags ?? []);

  return (
    <div className="space-y-6">
      {/* Create Todo Card */}
      <CreateTodoCard
        text={text}
        setText={setText}
        priority={priority}
        setPriority={setPriority}
        dueAt={dueAt}
        setDueAt={setDueAt}
        leadMinutes={leadMinutes}
        setLeadMinutes={setLeadMinutes}
        remindRoomId={remindRoomId}
        setRemindRoomId={setRemindRoomId}
        tags={tags}
        setTags={setTags}
        recurrence={recurrence}
        setRecurrence={setRecurrence}
        rooms={rooms}
        isPending={createMutation.isPending}
        errorMsg={errorMsg}
        onSubmit={handleCreate}
        textInputRef={textInputRef}
      />

      {/* Task List Table View */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400 whitespace-nowrap">Status:</span>
            <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono">
              {(['all', 'active', 'completed'] as const).map((tab) => (
                <button
                  key={tab}
                  onClick={() => setFilter(tab)}
                  className={`px-2.5 py-1 rounded capitalize transition cursor-pointer ${
                    filter === tab
                      ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                      : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
                  }`}
                >
                  {tab}
                </button>
              ))}
            </div>
            <label className="flex items-center gap-2">
              <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400 whitespace-nowrap">Tag:</span>
              <select
                aria-label="Filter by tag"
                value={tagFilter}
                onChange={(e) => setTagFilter(e.target.value)}
                className="h-7 px-2 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-neutral-700 dark:text-neutral-300 focus:outline-none focus-visible:border-neutral-500 transition-colors duration-150 cursor-pointer"
              >
                <option value="">All tags</option>
                {tagOptions.map((t) => (
                  <option key={t.tag} value={t.tag}>
                    #{t.tag}
                    {t.count > 0 ? ` (${t.count})` : ''}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              onClick={() =>
                triggerAiPrompt(
                  'Analyze my pending tasks, deadlines, and priorities. Recommend the top 3 items I should focus on first today and why.'
                )
              }
              className="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-sans font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 bg-white dark:bg-neutral-900 hover:bg-neutral-100 dark:hover:bg-neutral-800 border border-neutral-200 dark:border-neutral-800 rounded transition-colors shadow-2xs cursor-pointer"
              title="Ask AI to prioritize tasks"
            >
              <Sparkles className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
              <span>AI Prioritize</span>
            </button>
          </div>
          <span className="text-xs font-mono text-neutral-500 shrink-0">
            {filteredItems.length} items
          </span>
        </div>

        {isLoading ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500">Loading tasks...</div>
        ) : filteredItems.length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
            No tasks found. Create one above!
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {filteredItems.map((item) => (
              <TodoRow
                key={item.id}
                item={item}
                rooms={rooms ?? []}
                onToggle={() => updateMutation.mutate({ id: item.id, completed: !item.completed })}
                onDelete={() => deleteMutation.mutate(item.id)}
                onSave={(patch) => editMutation.mutateAsync({ id: item.id, ...patch })}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export const todoModule: FrontendModule = {
  id: 'todo',
  name: 'Tasks & Todos',
  description: 'Manage tasks with priorities, matrix room binding, and status tracking',
  navTab: { label: 'Todos' },
  MainView: TodoMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'TodoCreated' ||
      event.type === 'TodoUpdated' ||
      event.type === 'TodoDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['todos'] });
      queryClient.invalidateQueries({ queryKey: ['todo-tags'] });
      queryClient.invalidateQueries({ queryKey: ['todo-detail'] });
    }
  },
};
