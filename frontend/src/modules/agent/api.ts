import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { useSensitiveAction } from '../../core/security';
import {
  AgentRun,
  AgentRunDetail,
  ApprovalRequest,
  RejectActionRequest,
  ScheduledTask,
  ScheduleTaskRequest,
  StartAgentRunRequest,
} from './types';

export const AGENT_KEYS = {
  runs: ['agent-runs'] as const,
  runDetail: (id: string) => ['agent-run-detail', id] as const,
  approvals: ['agent-approvals'] as const,
  tasks: ['agent-tasks'] as const,
};

export function useAgentRuns(limit: number = 50) {
  return useQuery<AgentRun[]>({
    queryKey: AGENT_KEYS.runs,
    queryFn: () => fetchApi<AgentRun[]>(`/agent/runs?limit=${limit}`),
    refetchInterval: (query) => {
      // If any run is in an active state, poll every 3 seconds as a backstop to SSE
      const runs = query.state.data;
      const hasActive = runs?.some(
        (r) => r.status === 'running' || r.status === 'planning' || r.status === 'waiting_approval'
      );
      return hasActive ? 3000 : false;
    },
  });
}

export function useAgentRunDetail(runId: string | null) {
  return useQuery<AgentRunDetail>({
    queryKey: runId ? AGENT_KEYS.runDetail(runId) : ['agent-run-detail', 'none'],
    queryFn: () => fetchApi<AgentRunDetail>(`/agent/runs/${runId}`),
    enabled: Boolean(runId),
    refetchInterval: (query) => {
      const run = query.state.data?.run;
      const isActive =
        run?.status === 'running' || run?.status === 'planning' || run?.status === 'waiting_approval';
      return isActive ? 2000 : false;
    },
  });
}

export function useAgentApprovals() {
  return useQuery<ApprovalRequest[]>({
    queryKey: AGENT_KEYS.approvals,
    queryFn: () => fetchApi<ApprovalRequest[]>('/agent/approvals'),
  });
}

export function useStartRun() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body: StartAgentRunRequest) =>
      fetchApi<AgentRun>('/agent/runs', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.runs });
    },
  });
}

export function useCancelRun() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runId: string) =>
      fetchApi<AgentRun>(`/agent/runs/${runId}/cancel`, {
        method: 'POST',
      }),
    onSuccess: (updated) => {
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.runs });
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.runDetail(updated.id) });
    },
  });
}

export function useApproveAction() {
  return useSensitiveAction<{ id: string }, ApprovalRequest>(
    ({ id }) =>
      fetchApi<ApprovalRequest>(`/agent/approvals/${id}/approve`, {
        method: 'POST',
      }),
    [AGENT_KEYS.runs, AGENT_KEYS.approvals]
  );
}

export function useRejectAction() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, reason }: { id: string; reason?: string }) => {
      const body: RejectActionRequest = { reason: reason || null };
      return fetchApi<ApprovalRequest>(`/agent/approvals/${id}/reject`, {
        method: 'POST',
        body: JSON.stringify(body),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.runs });
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.approvals });
    },
  });
}

export function useScheduledTasks() {
  return useQuery<ScheduledTask[]>({
    queryKey: AGENT_KEYS.tasks,
    queryFn: () => fetchApi<ScheduledTask[]>('/agent/tasks'),
  });
}

export function useUpdateScheduledTask() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: ScheduleTaskRequest }) =>
      fetchApi<ScheduledTask>(`/agent/tasks/${encodeURIComponent(id)}`, {
        method: 'PUT',
        body: JSON.stringify(data),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: AGENT_KEYS.tasks });
    },
  });
}
