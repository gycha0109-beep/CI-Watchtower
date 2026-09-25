import { invoke } from '@tauri-apps/api/core';
import type {
  Dashboard,
  DeferResponsibilityDriftInput,
  DynamicWorkflowRuleInput,
  ProjectInput,
  ProjectWorkflowRuleInput,
  RepositoryInput,
  ResolveResponsibilityDriftInput,
  ResponsibilityEscalationOperatorInput,
  ResponsibilityEscalationOperatorResult,
  ResponsibilityResolutionAuditEntry,
  ResponsibilityResolutionPreview,
  ResponsibilityResolutionPreviewInput,
  ResponsibilityResolutionResult,
  ResponsibilityReviewPolicy,
  RunAttributionDetail,
  Settings,
  TrackInput,
} from './types';

export const api = {
  getDashboard: () => invoke<Dashboard>('get_dashboard'),
  acknowledgeResponsibilityEscalation: (input: ResponsibilityEscalationOperatorInput) =>
    invoke<ResponsibilityEscalationOperatorResult>('acknowledge_responsibility_escalation', { input }),
  suppressResponsibilityEscalation: (input: ResponsibilityEscalationOperatorInput) =>
    invoke<ResponsibilityEscalationOperatorResult>('suppress_responsibility_escalation', { input }),
  activateResponsibilityEscalation: (input: ResponsibilityEscalationOperatorInput) =>
    invoke<ResponsibilityEscalationOperatorResult>('activate_responsibility_escalation', { input }),
  getResponsibilityResolutionPreview: (input: ResponsibilityResolutionPreviewInput) =>
    invoke<ResponsibilityResolutionPreview>('get_responsibility_resolution_preview', { input }),
  resolveResponsibilityDrift: (input: ResolveResponsibilityDriftInput) =>
    invoke<ResponsibilityResolutionResult>('resolve_responsibility_drift', { input }),
  deferResponsibilityDrift: (input: DeferResponsibilityDriftInput) =>
    invoke<ResponsibilityResolutionResult>('defer_responsibility_drift', { input }),
  reopenResponsibilityDrift: (input: DeferResponsibilityDriftInput) =>
    invoke<ResponsibilityResolutionResult>('reopen_responsibility_drift', { input }),
  getResponsibilityResolutionHistory: () =>
    invoke<ResponsibilityResolutionAuditEntry[]>('get_responsibility_resolution_history'),
  getRunAttribution: (runId: number) =>
    invoke<RunAttributionDetail>('get_run_attribution', { runId }),
  pollNow: () => invoke<Dashboard>('poll_now'),
  saveProject: (input: ProjectInput) => invoke<number>('save_project', { input }),
  deleteProject: (id: number) => invoke<void>('delete_project', { id }),
  saveProjectWorkflowRule: (input: ProjectWorkflowRuleInput) =>
    invoke<number>('save_project_workflow_rule', { input }),
  deleteProjectWorkflowRule: (id: number) =>
    invoke<void>('delete_project_workflow_rule', { id }),
  saveDynamicWorkflowRule: (input: DynamicWorkflowRuleInput) =>
    invoke<number>('save_dynamic_workflow_rule', { input }),
  deleteDynamicWorkflowRule: (id: number) =>
    invoke<void>('delete_dynamic_workflow_rule', { id }),
  saveTrack: (input: TrackInput) => invoke<number>('save_track', { input }),
  deleteTrack: (id: number) => invoke<void>('delete_track', { id }),
  saveRepository: (input: RepositoryInput) => invoke<number>('save_repository', { input }),
  deleteRepository: (id: number) => invoke<void>('delete_repository', { id }),
  assignRunToProject: (runId: number, projectId: number, learnRule = true) =>
    invoke<void>('assign_run_to_project', { runId, projectId, learnRule }),
  assignRun: (runId: number, trackId: number) => invoke<void>('assign_run', { runId, trackId }),
  ignoreRun: (runId: number) => invoke<void>('ignore_run', { runId }),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  saveResponsibilityReviewPolicy: (policy: ResponsibilityReviewPolicy) =>
    invoke<void>('save_responsibility_review_policy', { policy }),
  setGithubToken: (token: string) => invoke<void>('set_github_token', { token }),
  clearGithubToken: () => invoke<void>('clear_github_token'),
  openExternal: (url: string) => invoke<void>('open_external', { url }),
};
