import { components } from '../../types/api';

export type AgentRun = components['schemas']['AgentRunResponse'];
export type AgentStep = components['schemas']['AgentStepResponse'];
export type AgentRunDetail = components['schemas']['AgentRunDetailResponse'];
export type ApprovalRequest = components['schemas']['ApprovalRequestResponse'];
export type ScheduledTask = components['schemas']['ScheduledTaskResponse'];
export type StartAgentRunRequest = components['schemas']['StartAgentRunRequest'];
export type RejectActionRequest = components['schemas']['RejectActionRequest'];
export type ScheduleTaskRequest = components['schemas']['ScheduleTaskRequest'];

export type AgentRunStatus =
  | 'planning'
  | 'running'
  | 'waiting_approval'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface AgentFilter {
  status?: AgentRunStatus | 'all';
}
