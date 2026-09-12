import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { useSession } from '../../core/session';
import { FrontendModule, SystemEvent } from '../../core/types';
import { Activity, Radio, Database, Server } from 'lucide-react';

interface HealthResponse {
  status: string;
  service: string;
  version: string;
  db_status: string;
}

export const CoreMainView: React.FC<{ events?: SystemEvent[]; isConnected?: boolean }> = ({
  events = [],
  isConnected = false,
}) => {
  const { isSignedIn: hasToken } = useSession();

  const { data: health } = useQuery<HealthResponse>({
    queryKey: ['health'],
    queryFn: () => fetchApi<HealthResponse>('/health'),
    refetchInterval: 10000,
  });

  return (
    <div className="space-y-6">
      {/* System Overview Grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-2">
          <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-medium">
            <span>REST API service</span>
            <Server className="w-4 h-4 text-emerald-500 dark:text-emerald-400" />
          </div>
          <div className="text-xl font-medium tracking-tight text-neutral-900 dark:text-neutral-100">
            {health?.service || 'matrix-bot-api'}
          </div>
          <div className="flex items-center gap-2 text-xs text-neutral-500 dark:text-neutral-400">
            <span className="inline-block w-2 h-2 rounded-full bg-emerald-500" />
            <span>Version {health?.version || '0.1.0'}</span>
          </div>
        </div>

        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-2">
          <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-medium">
            <span>SQLite database</span>
            <Database className="w-4 h-4 text-neutral-500" />
          </div>
          <div className="text-xl font-medium tracking-tight text-neutral-900 dark:text-neutral-100 capitalize">
            {health?.db_status || 'checking...'}
          </div>
          <div className="text-xs text-neutral-500">WAL journal mode · SQLite3</div>
        </div>

        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-2">
          <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-medium">
            <span>SSE broadcaster</span>
            <Radio className={`w-4 h-4 ${!hasToken ? 'text-neutral-400 dark:text-neutral-500' : isConnected ? 'text-emerald-500 dark:text-emerald-400' : 'text-amber-500 dark:text-amber-400'}`} />
          </div>
          <div className="text-xl font-medium tracking-tight text-neutral-900 dark:text-neutral-100">
            {!hasToken ? 'Auth Required' : isConnected ? 'Stream Active' : 'Connecting...'}
          </div>
          <div className="text-xs text-neutral-500 dark:text-neutral-400">
            {!hasToken ? 'Login to listen for real-time events' : 'Real-time system events'}
          </div>
        </div>
      </div>

      {/* Real-time System Event Audit Feed */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Activity className="w-4 h-4 text-neutral-500 dark:text-neutral-400" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">System Event Feed</h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">{events.length} events logged</span>
        </div>

        {events.length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
            {!hasToken ? 'Please click "Login / Token" to view real-time system events.' : 'Listening for live Matrix & REST API events...'}
          </div>
        ) : (
          <div className="space-y-2 max-h-96 overflow-y-auto pr-1">
            {events.map((ev, i) => (
              <div
                key={i}
                className="flex items-start justify-between p-3 rounded bg-white dark:bg-neutral-900/60 border border-neutral-200 dark:border-neutral-800 text-xs font-mono transition hover:border-neutral-300 dark:hover:border-neutral-700"
              >
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200">
                      {ev.type}
                    </span>
                    <span className="text-neutral-500 dark:text-neutral-400">{ev.timestamp}</span>
                  </div>
                  <pre className="text-neutral-800 dark:text-neutral-300 text-[11px] overflow-x-auto max-w-xl">
                    {JSON.stringify(ev.payload, null, 2)}
                  </pre>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export const coreModule: FrontendModule = {
  id: 'core',
  name: 'System Core',
  description: 'System health, SQLite status, and real-time SSE stream',
  navTab: { label: 'System Overview' },
  MainView: CoreMainView,
};
