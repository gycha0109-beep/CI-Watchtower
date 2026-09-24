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

export interface RunAttributionEvidence {
  trackKey: string;
  signalType: string;
  score: number;
  value: string;
  createdAt: string;
}

export interface ReconciliationEvidence {
  trackKey: string;
  signalType: string;
  score: number;
  value: string;
}

export interface ReconciliationAuditEntry {
  id: number;
  trigger: string;
  fromStatus: string;
  fromTrackKey: string | null;
  fromSource: string | null;
  fromConfidence: number | null;
  toStatus: string;
  toTrackKey: string | null;
  toSource: string | null;
  toConfidence: number | null;
  toReason: string | null;
  previousEvidence: ReconciliationEvidence[];
  evidence: ReconciliationEvidence[];
  reconciledAt: string;
}

export interface RunAttributionDetail {
  runId: number;
  projectId: number;
  repositoryId: number;
  repository: string;
  workflowName: string;
  resolutionStatus: string;
  assignedTrackId: number | null;
  assignedTrackName: string | null;
  assignedTrackKey: string | null;
  source: string | null;
  reason: string | null;
  confidence: number | null;
  manual: boolean | null;
  lastResolutionAttemptAt: string | null;
  projectRuleId: number | null;
  projectRuleRepositoryId: number | null;
  evidence: RunAttributionEvidence[];
  reconciliationHistory: ReconciliationAuditEntry[];
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

export interface ProducerContractStats {
  repositoryId: number;
  projectId: number;
  sampledRuns: number;
  projectWideRuns: number;
  explicitRuns: number;
  runNameRuns: number;
  prMarkerRuns: number;
  commitMarkerRuns: number;
  branchRuns: number;
  heuristicRuns: number;
  manualRuns: number;
  compatibilityRuns: number;
  unresolvedRuns: number;
  otherRuns: number;
}

export interface ProducerContractRun {
  run: WorkflowRunSummary;
  bucket: 'project' | 'run_name' | 'pr_marker' | 'commit_marker' | 'branch' | 'inference' | 'manual' | 'track_alias' | 'unassigned' | 'conflict' | 'other' | string;
  contractCompliant: boolean;
  isCurrentProducerRun: boolean;
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
  producerContractStats: ProducerContractStats[];
  producerContractRuns: ProducerContractRun[];
  projectRuns: WorkflowRunSummary[];
  unassignedRuns: WorkflowRunSummary[];
}
