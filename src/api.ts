import { invoke } from '@tauri-apps/api/core';
import type {
  Dashboard,
  ProjectInput,
  ProjectWorkflowRuleInput,
  RepositoryInput,
  Settings,
  TrackInput,
} from './types';

export const api = {
  getDashboard: () => invoke<Dashboard>('get_dashboard'),
  pollNow: () => invoke<Dashboard>('poll_now'),
  saveProject: (input: ProjectInput) => invoke<number>('save_project', { input }),
  deleteProject: (id: number) => invoke<void>('delete_project', { id }),
  saveProjectWorkflowRule: (input: ProjectWorkflowRuleInput) =>
    invoke<number>('save_project_workflow_rule', { input }),
  deleteProjectWorkflowRule: (id: number) =>
    invoke<void>('delete_project_workflow_rule', { id }),
  saveTrack: (input: TrackInput) => invoke<number>('save_track', { input }),
  deleteTrack: (id: number) => invoke<void>('delete_track', { id }),
  saveRepository: (input: RepositoryInput) => invoke<number>('save_repository', { input }),
  deleteRepository: (id: number) => invoke<void>('delete_repository', { id }),
  assignRunToProject: (runId: number, projectId: number, learnRule = true) =>
    invoke<void>('assign_run_to_project', { runId, projectId, learnRule }),
  assignRun: (runId: number, trackId: number) => invoke<void>('assign_run', { runId, trackId }),
  ignoreRun: (runId: number) => invoke<void>('ignore_run', { runId }),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  setGithubToken: (token: string) => invoke<void>('set_github_token', { token }),
  clearGithubToken: () => invoke<void>('clear_github_token'),
  openExternal: (url: string) => invoke<void>('open_external', { url }),
};
