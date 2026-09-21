import { invoke } from '@tauri-apps/api/core';
import type { Dashboard, Settings, TrackInput } from './types';

export const api = {
  getDashboard: () => invoke<Dashboard>('get_dashboard'),
  pollNow: () => invoke<Dashboard>('poll_now'),
  saveTrack: (input: TrackInput) => invoke<number>('save_track', { input }),
  deleteTrack: (id: number) => invoke<void>('delete_track', { id }),
  unarchiveAll: () => invoke<void>('unarchive_all'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  setGithubToken: (token: string) => invoke<void>('set_github_token', { token }),
  clearGithubToken: () => invoke<void>('clear_github_token'),
  openExternal: (url: string) => invoke<void>('open_external', { url }),
};
