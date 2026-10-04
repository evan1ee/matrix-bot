import React, { useState } from 'react';
import { FrontendModule, SystemEvent } from '../../core/types';
import { ApprovalBanner } from './components/ApprovalBanner';
import { RunList } from './components/RunList';
import { ScheduledTasksTab } from './components/ScheduledTasksTab';
import { AGENT_KEYS } from './api';
import { QueryClient } from '@tanstack/react-query';

const AgentMainView: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'runs' | 'tasks'>('runs');

  return (
    <div className="space-y-6">
      {/* Top Banner for Sensitive Tool Approvals */}
      <ApprovalBanner />

      {/* Navigation Sub-Tabs */}
      <div className="flex items-center gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-2">
        <button
          type="button"
          onClick={() => setActiveTab('runs')}
          className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
            activeTab === 'runs'
              ? 'bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800'
          }`}
        >
          Agent Runs
        </button>
        <button
          type="button"
          onClick={() => setActiveTab('tasks')}
          className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
            activeTab === 'tasks'
              ? 'bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800'
          }`}
        >
          Scheduled Tasks
        </button>
      </div>

      {/* Tab Panels */}
      {activeTab === 'runs' ? <RunList /> : <ScheduledTasksTab />}
    </div>
  );
};

export const agentModule: FrontendModule = {
  id: 'agent',
  name: 'Autonomous Agent',
  description: 'Orchestrate autonomous goal runs, approve sensitive tools, and schedule recurring tasks',
  iconName: 'Bot',
  navTab: {
    label: 'Agent',
  },
  MainView: AgentMainView,
  handleSseEvent: (event: SystemEvent, queryClient: QueryClient) => {
    if (event.type.startsWith('Agent')) {
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.runs });
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.approvals });
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.tasks });

      const payload = event.payload as Record<string, unknown> | undefined;
      if (payload?.run_id) {
        queryClient.invalidateQueries({
          queryKey: AGENT_KEYS.runDetail(payload.run_id as string),
        });
      }
    }
  },
};
