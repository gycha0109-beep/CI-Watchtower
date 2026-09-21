import { invoke } from '@tauri-apps/api/core';
import type { Dashboard, RepositoryInput, Settings, TrackInput } from './types';

export const api = {
  getDashboard: () => invoke<Dashboard>('get_dashboard'),
  pollNow: () => invoke<Dashboard>('poll_now'),
  saveTrack: (input: TrackInput) => invoke<number>('save_track', { input }),
  deleteTrack: (id: number) => invoke<void>('delete_track', { id }),
  saveRepository: (input: RepositoryInput) => invoke<number>('save_repository', { input }),
  deleteRepository: (id: number) => invoke<void>('delete_repository', { id }),
  assignRun: (runId: number, trackId: number) => invoke<void>('assign_run', { runId, trackId }),
  ignoreRun: (runId: number) => invoke<void>('ignore_run', { runId }),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  setGithubToken: (token: string) => invoke<void>('set_github_token', { token }),
  clearGithubToken: () => invoke<void>('clear_github_token'),
  openExternal: (url: string) => invoke<void>('open_external', { url }),
};
