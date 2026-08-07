import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import { CheckSquare, Plus, Trash2, CheckCircle2, Circle, AlertCircle } from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';

export interface TodoItem {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  text: string;
  priority: string;
  due_at?: string | null;
  completed: boolean;
  created_at?: string | null;
}

export interface TodoListResponse {
  items: TodoItem[];
  page: number;
  limit: number;
  total: number;
}

export const TodoMainView: React.FC = () => {
  const queryClient = useQueryClient();
  const [text, setText] = useState('');
  const [priority, setPriority] = useState('medium');
  const [filter, setFilter] = useState<'all' | 'active' | 'completed'>('all');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data, isLoading } = useQuery<TodoListResponse>({
    queryKey: ['todos'],
    queryFn: () => fetchApi<TodoListResponse>('/todos?limit=100'),
  });

  const createMutation = useMutation({
    mutationFn: (newTodo: { room_id: string; text: string; priority: string }) =>
      fetchApi<TodoItem>('/todos', {
        method: 'POST',
        body: JSON.stringify(newTodo),
      }),
    onSuccess: () => {
      setText('');
      setErrorMsg(null);
      queryClient.invalidateQueries({ queryKey: ['todos'] });
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
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['todos'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/todos/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['todos'] });
    },
  });

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    if (text.length > 500) {
      setErrorMsg('Task text exceeds 500 character limit.');
      return;
    }
    createMutation.mutate({ room_id: 'web', text: text.trim(), priority });
  };

  const filteredItems = (data?.items || []).filter((item) => {
    if (filter === 'active') return !item.completed;
    if (filter === 'completed') return item.completed;
    return true;
  });

  return (
    <div className="space-y-6">
      {/* Create Todo Card */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <CheckSquare className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Create New Task</h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">Max 500 chars</span>
        </div>

        {errorMsg && (
          <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 text-red-500 dark:text-red-400 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        <form onSubmit={handleCreate} className="space-y-3">
          <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
            <input
              type="text"
              placeholder="What needs to be done?"
              value={text}
              onChange={(e) => setText(e.target.value)}
              className="sm:col-span-2 px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
            />

            <CustomSelect
              value={priority}
              onChange={(val) => setPriority(val)}
              mono
              options={[
                { value: 'low', label: 'Low Priority' },
                { value: 'medium', label: 'Medium Priority' },
                { value: 'high', label: 'High Priority' },
              ]}
            />

            <button
              type="submit"
              disabled={createMutation.isPending}
              className="inline-flex items-center justify-center gap-2 px-4 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
            >
              <Plus className="w-4 h-4" />
              <span>{createMutation.isPending ? 'Adding...' : 'Add Task'}</span>
            </button>
          </div>
        </form>
      </div>

      {/* Task List Table View */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400">STATUS FILTER</span>
            <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono">
              {(['all', 'active', 'completed'] as const).map((tab) => (
                <button
                  key={tab}
                  onClick={() => setFilter(tab)}
                  className={`px-2.5 py-1 rounded capitalize transition ${
                    filter === tab
                      ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                      : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
                  }`}
                >
                  {tab}
                </button>
              ))}
            </div>
          </div>
          <span className="text-xs font-mono text-neutral-500">
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
              <div
                key={item.id}
                className="py-3 flex items-center justify-between gap-4 transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30 px-2 rounded"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <button
                    onClick={() => updateMutation.mutate({ id: item.id, completed: !item.completed })}
                    className="text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 transition shrink-0"
                  >
                    {item.completed ? (
                      <CheckCircle2 className="w-5 h-5 text-emerald-500 dark:text-emerald-400" />
                    ) : (
                      <Circle className="w-5 h-5 text-neutral-400 dark:text-neutral-600" />
                    )}
                  </button>
                  <div className="space-y-0.5 truncate">
                    <p
                      className={`text-sm font-medium ${
                        item.completed ? 'line-through text-neutral-400 dark:text-neutral-500' : 'text-neutral-900 dark:text-neutral-100'
                      }`}
                    >
                      {item.text}
                    </p>
                    <div className="flex items-center gap-2 text-xs font-mono text-neutral-500">
                      <span>by {item.sender_name}</span>
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-3 shrink-0">
                  <span
                    className={`px-2 py-0.5 rounded text-[10px] uppercase font-mono font-medium border ${
                      item.priority === 'high'
                        ? 'bg-rose-100 dark:bg-rose-950/40 text-rose-700 dark:text-rose-300 border-rose-300 dark:border-rose-900/50'
                        : item.priority === 'low'
                        ? 'bg-neutral-100 dark:bg-neutral-900 text-neutral-600 dark:text-neutral-400 border-neutral-200 dark:border-neutral-800'
                        : 'bg-amber-100 dark:bg-amber-950/40 text-amber-800 dark:text-amber-300 border-amber-300 dark:border-amber-900/50'
                    }`}
                  >
                    {item.priority}
                  </span>
                  <button
                    onClick={() => deleteMutation.mutate(item.id)}
                    className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
                    title="Delete task"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
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
    }
  },
};
