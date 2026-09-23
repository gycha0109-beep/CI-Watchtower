export interface Project {
  id: number;
  name: string;
  projectKey: string;
  active: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectInput {
  id?: number;
  name: string;
  projectKey: string;
}

export interface ProjectWorkflowRule {
  id: number;
  projectId: number;
  repositoryId: number | null;
  workflowName: string;
  active: boolean;
}

export interface ProjectWorkflowRuleInput {
  projectId: number;
  repositoryId: number | null;
  workflowName: string;
}

export interface Track {
  id: number;
  projectId: number;
  name: string;
  trackKey: string;
  longCiMinutes: number;
  active: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface TrackInput {
  id?: number;
  projectId: number;
  name: string;
  trackKey: string;
  longCiMinutes: number;
}

export interface MonitoredRepository {
  id: number;
  projectId: number;
  repo: string;
  enabled: boolean;
  runningCount: number;
  queuedCount: number;
  lastPolledAt: string | null;
  lastError: string | null;
}

export interface RepositoryInput {
  id?: number;
  projectId: number;
  repo: string;
  enabled: boolean;
}

export interface WorkflowRunSummary {
  id: number;
  projectId: number;
  repositoryId: number;
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
  resolutionStatus: 'assigned' | 'project' | 'unassigned' | 'conflict' | 'ignored' | string;
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

export interface RepositoryScopeStats {
  repositoryId: number;
  projectId: number;
  unassignedCount: number;
  projectRunCount: number;
}

export interface Dashboard {
  runningCount: number;
  queuedCount: number;
  unassignedCount: number;
  congestionLevel: 'safe' | 'busy' | 'congested';
  tokenConfigured: boolean;
  settings: Settings;
  projects: Project[];
  repositories: MonitoredRepository[];
  tracks: DashboardTrack[];
  projectWorkflowRules: ProjectWorkflowRule[];
  repositoryScopeStats: RepositoryScopeStats[];
  projectRuns: WorkflowRunSummary[];
  unassignedRuns: WorkflowRunSummary[];
}
