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

export interface DynamicWorkflowRule {
  id: number;
  projectId: number;
  repositoryId: number | null;
  workflowName: string;
  active: boolean;
  protected: boolean;
}

export interface DynamicWorkflowRuleInput {
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
  responsibilityDeclared: boolean;
  isCurrentProducerRun: boolean;
}

export interface ResponsibilityMapDrift {
  projectId: number;
  repositoryId: number;
  repository: string;
  workflowPath: string;
  workflowName: string;
  driftType: 'missing_in_watchtower' | 'stale_in_watchtower' | 'responsibility_kind_mismatch' | 'track_binding_mismatch' | string;
  repositoryBinding: string;
  watchtowerBinding: string | null;
  expectedTrackKey: string | null;
  actualTrackKey: string | null;
  sourcePath: string;
  reason: string;
  recommendedAction: string;
  reviewKey: string;
  fingerprint: string;
  reviewStatus: 'open' | 'deferred' | 'attention' | 'blocked' | string;
  reviewPriority: 'p0' | 'p1' | 'p2' | 'p3' | 'blocked' | string;
  reviewAgeBucket: 'fresh' | 'aging' | 'overdue' | string;
  reviewAgeHours: number;
  reviewEventCount: number;
  failedAttemptCount: number;
  lastReviewedAt: string | null;
  firstSeenAt: string;
  slaStatus: 'within_sla' | 'due_soon' | 'breached' | 'exempt' | string;
  slaTargetHours: number | null;
  slaRemainingHours: number | null;
  escalationLevel: 'critical' | 'warning' | 'none' | string;
  escalationReason: string | null;
}

export interface ResponsibilityResolutionPreviewInput {
  reviewKey: string;
}

export interface ResolveResponsibilityDriftInput {
  reviewKey: string;
  fingerprint: string;
  action: string;
}

export interface DeferResponsibilityDriftInput {
  reviewKey: string;
  fingerprint: string;
}

export interface ResponsibilityResolutionPreview {
  reviewKey: string;
  fingerprint: string;
  workflowName: string;
  repository: string;
  repositoryContract: string;
  watchtowerContract: string | null;
  action: string;
  changes: string[];
  invariants: string[];
  executable: boolean;
  blockedReason: string | null;
}

export interface ResponsibilityResolutionResult {
  status: 'resolved' | 'deferred' | 'blocked' | 'stale_rejected' | 'still_open' | string;
  auditId: number;
  currentDrift: ResponsibilityMapDrift | null;
}

export interface ResponsibilityResolutionAuditEntry {
  id: number;
  projectId: number;
  projectName: string;
  repositoryId: number;
  repository: string;
  workflowName: string;
  reviewKey: string;
  driftType: string;
  action: string;
  result: 'resolved' | 'deferred' | 'blocked' | 'stale_rejected' | 'still_open' | 'failed' | string;
  actor: string;
  createdAt: string;
  requestedFingerprint: string;
  currentFingerprint: string;
  repositoryContract: string;
  beforeWatchtowerContract: string | null;
  afterWatchtowerContract: string | null;
  stale: boolean;
}

export interface ResponsibilityMapSourceStatus {
  projectId: number;
  repositoryId: number;
  repository: string;
  sourcePath: string;
  status: 'pending' | 'synced' | 'not_found' | 'error' | string;
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  lastError: string | null;
  contractCount: number;
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
  dynamicWorkflowRules: DynamicWorkflowRule[];
  repositoryScopeStats: RepositoryScopeStats[];
  producerContractStats: ProducerContractStats[];
  producerContractRuns: ProducerContractRun[];
  responsibilityMapDrifts: ResponsibilityMapDrift[];
  responsibilityMapSources: ResponsibilityMapSourceStatus[];
  projectRuns: WorkflowRunSummary[];
  unassignedRuns: WorkflowRunSummary[];
}
