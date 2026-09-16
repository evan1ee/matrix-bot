import React, { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Check,
  Copy,
  Hash,
  Loader2,
  MessageSquare,
  RotateCw,
  Search,
  Terminal,
} from 'lucide-react';
import { fetchApi } from '../core/apiClient';
import { formatDateTimeInZone, useSettings, useUpdateSettings } from '../core/settings';
import { copyToClipboard } from '../core/clipboard';

export interface JoinedRoomInfo {
  room_id: string;
  name: string;
  is_direct?: boolean;
  updated_at?: string | null;
}

export const RoomsPanel: React.FC = () => {
  const queryClient = useQueryClient();
  const settings = useSettings();
  const { mutateAsync: updateSettings } = useUpdateSettings();

  const [filter, setFilter] = useState('');
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);

  const { data: rooms = [], isLoading } = useQuery<JoinedRoomInfo[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  const syncMutation = useMutation({
    mutationFn: () => fetchApi<JoinedRoomInfo[]>('/rooms/sync', { method: 'POST' }),
    onSuccess: (updatedRooms) => {
      setSyncError(null);
      queryClient.setQueryData(['rooms'], updatedRooms);
      queryClient.invalidateQueries({ queryKey: ['rooms'] });
    },
    onError: (err: Error) => {
      setSyncError(err.message || 'Failed to sync rooms from Matrix');
    },
  });

  const handleCopy = async (roomId: string) => {
    const ok = await copyToClipboard(roomId);
    if (ok) {
      setCopiedId(roomId);
      setTimeout(() => setCopiedId(null), 2000);
    }
  };

  const handleSetDevRoom = async (roomId: string) => {
    try {
      await updateSettings({
        timezone: settings.timezone,
        currency: settings.currency,
        dev_room_id: roomId === settings.dev_room_id ? null : roomId,
      });
    } catch {
      // Ignored: notification handled by settings
    }
  };

  const filteredRooms = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return rooms;
    return rooms.filter(
      (r) => r.name.toLowerCase().includes(q) || r.room_id.toLowerCase().includes(q)
    );
  }, [rooms, filter]);

  const lastUpdated = useMemo(() => {
    const ts = rooms.find((r) => r.updated_at)?.updated_at;
    if (!ts) return null;
    return formatDateTimeInZone(ts, settings.timezone);
  }, [rooms, settings.timezone]);

  return (
    <div className="space-y-4">
      {/* Header & Sync action */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-2">
        <div>
          <h4 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
            <span>Matrix Rooms</span>
            <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
              {rooms.length}
            </span>
          </h4>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-0.5">
            Rooms containing both the bot and your linked Matrix account. Cached in database for instant loading.
          </p>
          {lastUpdated && (
            <p className="text-[10px] font-mono text-neutral-400 dark:text-neutral-500 mt-0.5">
              Last synced: {lastUpdated}
            </p>
          )}
        </div>

        <button
          type="button"
          onClick={() => syncMutation.mutate()}
          disabled={syncMutation.isPending || isLoading}
          className="shrink-0 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
        >
          {syncMutation.isPending ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RotateCw className="w-3.5 h-3.5" />
          )}
          <span>{syncMutation.isPending ? 'Syncing...' : 'Sync from Matrix'}</span>
        </button>
      </div>

      {syncError && (
        <div className="p-3 rounded bg-rose-100 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800/50 text-rose-800 dark:text-rose-300 text-xs font-mono">
          {syncError}
        </div>
      )}

      {/* Search filter */}
      {rooms.length > 0 && (
        <div className="relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-neutral-400 pointer-events-none" />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Search rooms by name or ID..."
            className="w-full pl-9 pr-3 py-1.5 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-600 focus:outline-none focus:border-neutral-500 font-sans"
          />
        </div>
      )}

      {/* Room list */}
      <div className="max-h-64 overflow-y-auto rounded-lg border border-neutral-200 dark:border-neutral-800 divide-y divide-neutral-100 dark:divide-neutral-900">
        {isLoading ? (
          <div className="py-8 flex items-center justify-center gap-2 text-xs font-mono text-neutral-500">
            <Loader2 className="w-4 h-4 animate-spin" />
            <span>Loading cached rooms...</span>
          </div>
        ) : filteredRooms.length === 0 ? (
          <div className="py-8 px-4 text-center space-y-2">
            <p className="text-xs font-mono text-neutral-500">
              {filter ? 'No rooms match your filter.' : 'No rooms synced in database.'}
            </p>
            {!filter && (
              <p className="text-[11px] text-neutral-400">
                Click &quot;Sync from Matrix&quot; above to discover rooms shared between the bot and your linked Matrix account.
              </p>
            )}
          </div>
        ) : (
          filteredRooms.map((room) => {
            const isDevRoom = settings.dev_room_id === room.room_id;
            const isCopied = copiedId === room.room_id;

            return (
              <div
                key={room.room_id}
                className="p-3 flex items-center justify-between gap-3 hover:bg-neutral-50 dark:hover:bg-neutral-900/60 transition"
              >
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex items-center gap-2">
                    {room.is_direct ? (
                      <MessageSquare className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
                    ) : (
                      <Hash className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
                    )}
                    <span className="text-xs font-medium text-neutral-900 dark:text-neutral-100 truncate">
                      {room.name}
                    </span>

                    {isDevRoom && (
                      <span className="inline-flex items-center gap-1 text-[10px] font-mono px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20 shrink-0">
                        <Terminal className="w-2.5 h-2.5" />
                        Dev Room
                      </span>
                    )}
                  </div>

                  <div className="flex items-center gap-2">
                    <span className="text-[10px] font-mono text-neutral-400 dark:text-neutral-500 truncate select-all">
                      {room.room_id}
                    </span>
                  </div>
                </div>

                <div className="flex items-center gap-1.5 shrink-0">
                  <button
                    type="button"
                    onClick={() => handleCopy(room.room_id)}
                    title="Copy room ID"
                    className="p-1.5 rounded text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
                  >
                    {isCopied ? (
                      <Check className="w-3.5 h-3.5 text-emerald-500" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                  </button>

                  <button
                    type="button"
                    onClick={() => handleSetDevRoom(room.room_id)}
                    title={isDevRoom ? 'Remove as Dev Room' : 'Set as Dev Room'}
                    className={`text-[11px] font-mono px-2 py-1 rounded transition cursor-pointer ${
                      isDevRoom
                        ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-300 dark:hover:bg-neutral-700'
                        : 'border border-neutral-200 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-200'
                    }`}
                  >
                    {isDevRoom ? 'Active Dev' : 'Set as Dev'}
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
