export interface Track {
  id: number;
  name: string;
  trackKey: string;
  longCiMinutes: number;
  active: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface TrackInput {
  id?: number;
  name: string;
  trackKey: string;
  longCiMinutes: number;
}

export interface MonitoredRepository {
  id: number;
  repo: string;
  enabled: boolean;
  runningCount: number;
  queuedCount: number;
  lastPolledAt: string | null;
  lastError: string | null;
}

export interface RepositoryInput {
  id?: number;
  repo: string;
  enabled: boolean;
}

export interface WorkflowRunSummary {
  id: number;
  repository: string;
  workflowName: string;
  displayTitle: string;
  event: string;
  headBranch: string | null;
  headSha: string;
  runAttempt: number;
  status: string;
  conclusion: string | null;
  htmlUrl: string;
  createdAt: string;
  runStartedAt: string | null;
  updatedAt: string;
  elapsedSeconds: number;
  attributionSource: string | null;
  attributionReason: string | null;
  confidence: number | null;
  resolutionStatus: 'assigned' | 'unassigned' | 'conflict' | 'ignored' | string;
}

export type TrackHealth =
  | 'waiting'
  | 'queued'
  | 'running'
  | 'green'
  | 'red'
  | 'completed_other';

export interface DashboardTrack {
  track: Track;
  health: TrackHealth;
  elapsedSeconds: number;
  averageDurationSeconds: number | null;
  runs: WorkflowRunSummary[];
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
  unassignedCount: number;
  congestionLevel: 'safe' | 'busy' | 'congested';
  tokenConfigured: boolean;
  settings: Settings;
  repositories: MonitoredRepository[];
  tracks: DashboardTrack[];
  unassignedRuns: WorkflowRunSummary[];
}
