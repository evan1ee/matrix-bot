import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import {
  CheckSquare,
  Plus,
  Trash2,
  CheckCircle2,
  Circle,
  AlertCircle,
  CalendarClock,
  Bell,
  Pencil,
  Check,
  X,
} from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { DateTimePicker } from '../../components/DateTimePicker';
import {
  formatDateTimeInZone,
  nowInZone,
  useSettings,
  utcIsoToZonedInput,
  zonedInputToUtcIso,
} from '../../core/settings';

export interface TodoItem {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  text: string;
  priority: string;
  due_at?: string | null;
  remind_before_minutes?: number | null;
  completed: boolean;
  created_at?: string | null;
}

export interface TodoListResponse {
  items: TodoItem[];
  page: number;
  limit: number;
  total: number;
}

interface JoinedRoomInfo {
  room_id: string;
  name: string;
}

/** Lead-time presets, in minutes. `''` means no reminder. */
const LEAD_OPTIONS = [
  { value: '', label: 'No reminder' },
  { value: '10', label: '10 minutes before' },
  { value: '60', label: '1 hour before' },
  { value: '1440', label: '1 day before' },
  { value: '10080', label: '1 week before' },
];

function leadLabel(minutes: number): string {
  return LEAD_OPTIONS.find((o) => o.value === String(minutes))?.label ?? `${minutes} min before`;
}

/** The editable fields of a task, as the PATCH body expects them. */
interface TodoPatch {
  text: string;
  priority: string;
  due_at: string | null;
  remind_before_minutes: number | null;
  remind_room_id?: string;
}

interface TodoRowProps {
  item: TodoItem;
  rooms: JoinedRoomInfo[];
  onToggle: () => void;
  onDelete: () => void;
  onSave: (patch: TodoPatch) => Promise<unknown>;
}

const priorityBadgeClass = (priority: string) =>
  priority === 'high'
    ? 'bg-rose-100 dark:bg-rose-950/40 text-rose-700 dark:text-rose-300 border-rose-300 dark:border-rose-900/50'
    : priority === 'low'
    ? 'bg-neutral-100 dark:bg-neutral-900 text-neutral-600 dark:text-neutral-400 border-neutral-200 dark:border-neutral-800'
    : 'bg-amber-100 dark:bg-amber-950/40 text-amber-800 dark:text-amber-300 border-amber-300 dark:border-amber-900/50';

const TodoRow: React.FC<TodoRowProps> = ({ item, rooms, onToggle, onDelete, onSave }) => {
  const { timezone } = useSettings();
  const [editing, setEditing] = useState(false);
  const [draftText, setDraftText] = useState(item.text);
  const [draftPriority, setDraftPriority] = useState(item.priority);
  const [draftDue, setDraftDue] = useState(utcIsoToZonedInput(item.due_at, timezone));
  const [draftLead, setDraftLead] = useState(
    item.remind_before_minutes ? String(item.remind_before_minutes) : ''
  );
  const [draftRoom, setDraftRoom] = useState('');
  const [saving, setSaving] = useState(false);
  const [rowError, setRowError] = useState<string | null>(null);

  // The task already has a reminder, so the server knows which room it goes to
  // and the caller only needs to name one to *change* it.
  const hasReminder = Boolean(item.due_at && item.remind_before_minutes);
  const needsRoom = item.room_id === 'web';

  const beginEdit = () => {
    setDraftText(item.text);
    setDraftPriority(item.priority);
    setDraftDue(utcIsoToZonedInput(item.due_at, timezone));
    setDraftLead(item.remind_before_minutes ? String(item.remind_before_minutes) : '');
    setDraftRoom('');
    setRowError(null);
    setEditing(true);
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = draftText.trim();
    if (!trimmed) {
      setRowError('Task text cannot be empty.');
      return;
    }
    if (trimmed.length > 500) {
      setRowError('Task text exceeds 500 character limit.');
      return;
    }

    const dueIso = zonedInputToUtcIso(draftDue, timezone);
    const wantsReminder = draftLead !== '';
    if (wantsReminder && !dueIso) {
      setRowError('Pick a due date for the reminder to count back from.');
      return;
    }
    if (wantsReminder && needsRoom && !hasReminder && !draftRoom) {
      setRowError('Choose a Matrix room for the reminder to be sent to.');
      return;
    }

    setSaving(true);
    setRowError(null);
    try {
      await onSave({
        text: trimmed,
        priority: draftPriority,
        due_at: dueIso,
        remind_before_minutes: wantsReminder ? Number(draftLead) : null,
        // Omitted entirely when unchanged, so the server keeps the room the
        // existing reminder already uses.
        ...(draftRoom ? { remind_room_id: draftRoom } : {}),
      });
      setEditing(false);
    } catch (err) {
      setRowError(err instanceof Error ? err.message : 'Failed to save changes.');
    } finally {
      setSaving(false);
    }
  };

  if (editing) {
    return (
      <form
        onSubmit={submit}
        data-testid="todo-row-edit"
        className="py-3 px-2 space-y-3 bg-neutral-100/60 dark:bg-neutral-900/30 rounded"
      >
        <input
          type="text"
          value={draftText}
          onChange={(e) => setDraftText(e.target.value)}
          className="w-full px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
        />

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <label className="flex flex-col gap-1">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Priority</span>
            <CustomSelect
              value={draftPriority}
              onChange={setDraftPriority}
              mono
              options={[
                { value: 'low', label: 'Low Priority' },
                { value: 'medium', label: 'Medium Priority' },
                { value: 'high', label: 'High Priority' },
              ]}
            />
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Due date</span>
            <DateTimePicker
              value={draftDue}
              onChange={setDraftDue}
              placeholder="No due date"
              now={nowInZone(timezone)}
            />
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Remind me</span>
            <CustomSelect value={draftLead} onChange={setDraftLead} mono options={LEAD_OPTIONS} />
          </label>
        </div>

        {draftLead !== '' && needsRoom && (
          <label className="flex flex-col gap-1">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Send to room</span>
            <CustomSelect
              value={draftRoom}
              onChange={setDraftRoom}
              mono
              options={[
                { value: '', label: hasReminder ? 'Keep current room' : 'Select a room…' },
                ...rooms.map((r) => ({ value: r.room_id, label: r.name })),
              ]}
            />
          </label>
        )}

        {rowError && (
          <div className="flex items-center gap-2 p-2 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{rowError}</span>
          </div>
        )}

        <div className="flex items-center gap-2">
          <button
            type="submit"
            disabled={saving}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
          >
            <Check className="w-3.5 h-3.5" />
            {saving ? 'Saving…' : 'Save'}
          </button>
          <button
            type="button"
            onClick={() => setEditing(false)}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 text-neutral-600 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition"
          >
            <X className="w-3.5 h-3.5" />
            Cancel
          </button>
        </div>
      </form>
    );
  }

  return (
    <div
      data-testid="todo-row"
      className="py-3 flex items-center justify-between gap-4 transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30 px-2 rounded"
    >
      <div className="flex items-center gap-3 min-w-0">
        <button
          onClick={onToggle}
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
              item.completed
                ? 'line-through text-neutral-400 dark:text-neutral-500'
                : 'text-neutral-900 dark:text-neutral-100'
            }`}
          >
            {item.text}
          </p>
          <div className="flex items-center gap-2 flex-wrap text-xs font-mono text-neutral-500">
            <span>by {item.sender_name}</span>
            {item.due_at && (
              <span
                data-testid="due-date"
                data-overdue={
                  !item.completed && new Date(item.due_at) < new Date() ? 'true' : 'false'
                }
                className={`inline-flex items-center gap-1 ${
                  !item.completed && new Date(item.due_at) < new Date()
                    ? 'text-rose-600 dark:text-rose-400'
                    : ''
                }`}
                title={`Due ${formatDateTimeInZone(item.due_at, timezone)}`}
              >
                <CalendarClock className="w-3 h-3" />
                {formatDateTimeInZone(item.due_at, timezone)}
              </span>
            )}
            {item.due_at && item.remind_before_minutes ? (
              <span className="inline-flex items-center gap-1">
                <Bell className="w-3 h-3" />
                {leadLabel(item.remind_before_minutes)}
              </span>
            ) : null}
          </div>
        </div>
      </div>

      <div className="flex items-center gap-3 shrink-0">
        <span
          className={`px-2 py-0.5 rounded text-[10px] uppercase font-mono font-medium border ${priorityBadgeClass(
            item.priority
          )}`}
        >
          {item.priority}
        </span>
        <button
          onClick={beginEdit}
          className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
          title="Edit task"
        >
          <Pencil className="w-4 h-4" />
        </button>
        <button
          onClick={onDelete}
          className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
          title="Delete task"
        >
          <Trash2 className="w-4 h-4" />
        </button>
      </div>
    </div>
  );
};

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
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data, isLoading } = useQuery<TodoListResponse>({
    queryKey: ['todos'],
    queryFn: () => fetchApi<TodoListResponse>('/todos?limit=100'),
  });

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
  const noRoomsAvailable = wantsReminder && (rooms?.length ?? 0) === 0;

  const createMutation = useMutation({
    mutationFn: (newTodo: {
      room_id: string;
      text: string;
      priority: string;
      due_at: string | null;
      remind_before_minutes: number | null;
      remind_room_id: string | null;
    }) =>
      fetchApi<TodoItem>('/todos', {
        method: 'POST',
        body: JSON.stringify(newTodo),
      }),
    onSuccess: () => {
      setText('');
      setDueAt('');
      setLeadMinutes('');
      setErrorMsg(null);
      queryClient.invalidateQueries({ queryKey: ['todos'] });
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
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['todos'] });
    },
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
      queryClient.invalidateQueries({ queryKey: ['todos'] });
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/todos/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['todos'] });
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
    });
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
              ref={textInputRef}
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

          <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
            <label className="sm:col-span-2 flex flex-col gap-1">
              <span className="text-[10px] uppercase font-mono text-neutral-500">
                Due date (optional)
              </span>
              <DateTimePicker
                value={dueAt}
                onChange={setDueAt}
                placeholder="No due date"
                now={nowInZone(timezone)}
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Remind me</span>
              <CustomSelect
                value={leadMinutes}
                onChange={setLeadMinutes}
                mono
                options={LEAD_OPTIONS}
              />
            </label>

            {wantsReminder && (
              <label className="flex flex-col gap-1">
                <span className="text-[10px] uppercase font-mono text-neutral-500">Send to room</span>
                <CustomSelect
                  value={remindRoomId}
                  onChange={setRemindRoomId}
                  mono
                  options={[
                    { value: '', label: 'Select a room…' },
                    ...(rooms ?? []).map((r) => ({ value: r.room_id, label: r.name })),
                  ]}
                />
              </label>
            )}
          </div>

          {noRoomsAvailable && (
            <p className="text-xs font-mono text-amber-700 dark:text-amber-400">
              No Matrix rooms available. Reminders are delivered to a room you and the bot both
              belong to — sign in with your Matrix account to pick one.
            </p>
          )}
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
    }
  },
};
