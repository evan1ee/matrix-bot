import React, { useState, useEffect } from 'react';
import { CustomSelect } from '../../components/CustomSelect';
import { JoinedRoomInfo } from './types';

interface CreateWebhookModalProps {
  rooms: JoinedRoomInfo[];
  isPending: boolean;
  formError: string | null;
  onClose: () => void;
  onSubmit: (body: { name: string; default_room_id: string }) => void;
  setFormError: (err: string | null) => void;
}

export const CreateWebhookModal: React.FC<CreateWebhookModalProps> = ({
  rooms,
  isPending,
  formError,
  onClose,
  onSubmit,
  setFormError,
}) => {
  const [name, setName] = useState('');
  const [roomId, setRoomId] = useState('');

  useEffect(() => {
    if (rooms.length > 0 && !roomId) {
      setRoomId(rooms[0].room_id);
    }
  }, [rooms, roomId]);

  const roomOptions = rooms.map((r) => ({
    value: r.room_id,
    label: r.name || r.room_id,
  }));

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setFormError('Webhook name is required');
      return;
    }
    const targetRoom = roomId || (rooms.length > 0 ? rooms[0].room_id : '');
    if (!targetRoom) {
      setFormError('Please select a target Matrix room');
      return;
    }
    onSubmit({
      name: name.trim(),
      default_room_id: targetRoom,
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
    >
      <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-lg rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl animate-in fade-in-0 zoom-in-95">
        <div className="p-6 border-b border-neutral-200 dark:border-neutral-800">
          <h2 className="text-lg font-bold text-neutral-900 dark:text-neutral-100">Create Inbound Webhook</h2>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
            Generates a secure endpoint for external scripts and alert integrations.
          </p>
        </div>

        <form onSubmit={handleSubmit} className="p-6 space-y-4">
          {formError && (
            <div className="p-3 bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 rounded-lg text-red-800 dark:text-red-300 text-xs font-mono">
              {formError}
            </div>
          )}

          <div>
            <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1.5">
              Webhook Name <span className="text-red-500">*</span>
            </label>
            <input
              type="text"
              required
              placeholder="e.g. VPS Health Monitor, GitHub Actions, Uptime Kuma"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full h-9 px-3 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 rounded-md text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1.5">
              Target Matrix Room <span className="text-red-500">*</span>
            </label>
            <CustomSelect
              options={roomOptions}
              value={roomId || (rooms.length > 0 ? rooms[0].room_id : '')}
              onChange={(val) => setRoomId(val)}
              placeholder="Select destination room..."
              className="w-full"
            />
            <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-1">
              Incoming alerts sent to this webhook will be delivered to this room by default.
            </p>
          </div>

          <div className="pt-4 border-t border-neutral-200 dark:border-neutral-800 flex justify-end gap-2.5">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3.5 text-xs font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 border border-neutral-300 dark:border-neutral-700 hover:bg-neutral-100 dark:hover:bg-neutral-800 rounded-md transition-colors cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isPending}
              className="h-9 px-4 text-xs font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-md transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isPending ? 'Generating...' : 'Generate Webhook'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
