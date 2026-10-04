import React, { lazy } from 'react';
import type { FrontendModule, SystemEvent } from '../../core/types';
import { AGENT_KEYS } from './api';
import type { QueryClient } from '@tanstack/react-query';

const AgentMainView = lazy(() =>
  import('./AgentMainView').then((m) => ({ default: m.AgentMainView }))
);

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
