import React, { useState } from 'react';
import {
  CalendarClock,
  Bell,
  Pencil,
  Trash2,
  CheckCircle2,
  Circle,
  ChevronRight,
  ChevronDown,
} from 'lucide-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../../core/apiClient';
import { TagInput } from './TagInput';
import { RepeatSelect } from './RepeatSelect';
import { StepList } from './StepList';
import { CustomSelect } from '../../../components/CustomSelect';
import { DateTimePicker } from '../../../components/DateTimePicker';
import {
  formatDateTimeInZone,
  nowInZone,
  useSettings,
  utcIsoToZonedInput,
  zonedInputToUtcIso,
} from '../../../core/settings';
import {
  TodoItem,
  JoinedRoomInfo,
  TodoPatch,
  TodoDetail,
  LEAD_OPTIONS,
  leadLabel,
  priorityBadgeClass,
} from '../types';

export interface TodoRowProps {
  item: TodoItem;
  rooms: JoinedRoomInfo[];
  onToggle: () => void;
  onDelete: () => void;
  onSave: (patch: TodoPatch) => Promise<unknown>;
}

export const TodoRow: React.FC<TodoRowProps> = ({ item, rooms, onToggle, onDelete, onSave }) => {
  const { timezone } = useSettings();
  const [editing, setEditing] = useState(false);
  const [draftText, setDraftText] = useState(item.text);
  const [draftPriority, setDraftPriority] = useState(item.priority);
  const [draftDue, setDraftDue] = useState(utcIsoToZonedInput(item.due_at, timezone));
  const [draftLead, setDraftLead] = useState(
    item.remind_before_minutes ? String(item.remind_before_minutes) : ''
  );
  const [draftRoom, setDraftRoom] = useState('');
  const [draftTags, setDraftTags] = useState(item.tags);
  const [draftRecurrence, setDraftRecurrence] = useState(item.recurrence);
  const [expanded, setExpanded] = useState(false);
  const [saving, setSaving] = useState(false);
  const queryClient = useQueryClient();

  // Loaded lazily: the list endpoint only carries step counts.
  const { data: detail } = useQuery<TodoDetail>({
    queryKey: ['todo-detail', item.id],
    queryFn: () => fetchApi<TodoDetail>(`/todos/${item.id}`),
    enabled: expanded,
  });
  const onStepsChange = (next: TodoDetail) => {
    queryClient.setQueryData(['todo-detail', item.id], next);
    queryClient.invalidateQueries({ queryKey: ['todos'] });
  };
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
    setDraftTags(item.tags);
    setDraftRecurrence(item.recurrence);
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
        // Omitted when untouched: the input has minute precision, so re-sending
        // it would move a due date that carries seconds and re-anchor a repeat.
        ...(draftDue !== utcIsoToZonedInput(item.due_at, timezone) ? { due_at: dueIso } : {}),
        remind_before_minutes: wantsReminder ? Number(draftLead) : null,
        tags: draftTags,
        recurrence: dueIso ? draftRecurrence : 'none',
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
          className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
        />

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Priority</span>
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
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Due date</span>
            <DateTimePicker
              value={draftDue}
              onChange={setDraftDue}
              placeholder="No due date"
              now={nowInZone(timezone)}
            />
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Remind me</span>
            <CustomSelect value={draftLead} onChange={setDraftLead} mono options={LEAD_OPTIONS} />
          </label>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <div className="sm:col-span-2">
            <TagInput value={draftTags} onChange={setDraftTags} />
          </div>
          <RepeatSelect
            value={draftRecurrence}
            onChange={setDraftRecurrence}
            hasDueDate={draftDue !== ''}
          />
        </div>

        {draftLead !== '' && needsRoom && (
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Send to room</span>
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

        <div className="flex items-center justify-between pt-1">
          {rowError && (
            <span className="text-xs font-mono text-rose-600 dark:text-rose-400">{rowError}</span>
          )}
          <div className="flex items-center gap-2 ml-auto">
            <button
              type="button"
              onClick={() => setEditing(false)}
              className="px-3 py-1 text-xs rounded border border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-850 transition cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-3 py-1 text-xs rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
          </div>
        </div>
      </form>
    );
  }

  return (
    <div data-testid="todo-row">
      <div className="flex items-center justify-between p-3 rounded-md hover:bg-neutral-100 dark:hover:bg-neutral-900/50 transition border border-transparent hover:border-neutral-200 dark:hover:border-neutral-800/80 group">
        <div className="flex items-center gap-3 overflow-hidden mr-4">
          <button
            onClick={onToggle}
            className="text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 transition shrink-0 cursor-pointer"
          >
            {item.completed ? (
              <CheckCircle2 className="w-4 h-4 text-emerald-600 dark:text-emerald-500" />
            ) : (
              <Circle className="w-4 h-4" />
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
              {item.recurrence !== 'none' && <span>↻ {item.recurrence}</span>}
              {item.tags.map((tag) => (
                <span key={tag} className="text-neutral-700 dark:text-neutral-300">
                  #{tag}
                </span>
              ))}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-3 shrink-0">
          <button
            type="button"
            onClick={() => setExpanded((open) => !open)}
            aria-expanded={expanded}
            aria-label={expanded ? 'Hide checklist' : 'Show checklist'}
            className="inline-flex items-center gap-1 p-1.5 rounded text-xs font-mono text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition-colors duration-150 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-500"
          >
            {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
            {item.steps_total > 0 && (
              <span>
                {item.steps_done}/{item.steps_total}
              </span>
            )}
          </button>
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
      {expanded && detail && (
        <StepList todoId={item.id} steps={detail.steps} onChange={onStepsChange} />
      )}
    </div>
  );
};
