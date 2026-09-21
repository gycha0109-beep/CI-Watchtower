export type TrackSourceMode = 'branch' | 'pr';

export interface Track {
  id: number;
  name: string;
  repo: string;
  sourceMode: TrackSourceMode;
  branch: string | null;
  prNumber: number | null;
  workflowFilter: string | null;
  longCiMinutes: number;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface WorkflowRunSummary {
  id: number;
  name: string;
  status: string;
  conclusion: string | null;
  htmlUrl: string;
  createdAt: string;
  runStartedAt: string | null;
  updatedAt: string;
  elapsedSeconds: number;
}

export type TrackHealth =
  | 'waiting'
  | 'queued'
  | 'running'
  | 'green'
  | 'red'
  | 'completed_other'
  | 'error';

export interface TrackState {
  trackId: number;
  health: TrackHealth;
  headSha: string | null;
  prUrl: string | null;
  latestRunUrl: string | null;
  checkedAt: string | null;
  message: string | null;
  elapsedSeconds: number;
  averageDurationSeconds: number | null;
  runs: WorkflowRunSummary[];
}

export interface DashboardTrack {
  track: Track;
  state: TrackState;
}

export interface Settings {
  queueCongestionThreshold: number;
  activePollSeconds: number;
  idlePollSeconds: number;
  autoArchiveCompleted: boolean;
}

export interface Dashboard {
  runningCount: number;
  queuedCount: number;
  congestionLevel: 'safe' | 'busy' | 'congested';
  tokenConfigured: boolean;
  settings: Settings;
  tracks: DashboardTrack[];
}

export interface TrackInput {
  id?: number;
  name: string;
  repo: string;
  sourceMode: TrackSourceMode;
  branch?: string | null;
  prNumber?: number | null;
  workflowFilter?: string | null;
  longCiMinutes: number;
}
