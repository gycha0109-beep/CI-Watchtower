use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use keyring::Entry;
use reqwest::{header, Client};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;

const KEYRING_SERVICE: &str = "ci-watchtower";
const KEYRING_ACCOUNT: &str = "github-pat";
const DEFAULT_ACTIVE_POLL_SECONDS: i64 = 25;
const DEFAULT_IDLE_POLL_SECONDS: i64 = 90;
const DEFAULT_QUEUE_THRESHOLD: i64 = 6;
const HISTORICAL_RECONCILE_BATCH: i64 = 12;
const PRODUCER_CONTRACT_SAMPLE_PER_REPOSITORY: i64 = 50;
const RESPONSIBILITY_MAP_PATH: &str = "docs/ci/workflow-responsibility-map.json";
const RESPONSIBILITY_ESCALATION_MAX_ATTEMPTS: i64 = 3;
const DEFAULT_RESPONSIBILITY_P0_TARGET_HOURS: i64 = 24;
const DEFAULT_RESPONSIBILITY_P1_TARGET_HOURS: i64 = 96;
const DEFAULT_RESPONSIBILITY_P2_TARGET_HOURS: i64 = 72;
const DEFAULT_RESPONSIBILITY_P0_DUE_SOON_HOURS: i64 = 12;
const DEFAULT_RESPONSIBILITY_P1_DUE_SOON_HOURS: i64 = 24;
const DEFAULT_RESPONSIBILITY_P2_DUE_SOON_HOURS: i64 = 24;

struct AppState {
    db_path: PathBuf,
    poll_in_flight: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: i64,
    name: String,
    project_key: String,
    active: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInput {
    id: Option<i64>,
    name: String,
    project_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectWorkflowRule {
    id: i64,
    project_id: i64,
    repository_id: Option<i64>,
    workflow_name: String,
    active: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectWorkflowRuleInput {
    project_id: i64,
    repository_id: Option<i64>,
    workflow_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicWorkflowRule {
    id: i64,
    project_id: i64,
    repository_id: Option<i64>,
    workflow_name: String,
    active: bool,
    protected: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicWorkflowRuleInput {
    project_id: i64,
    repository_id: Option<i64>,
    workflow_name: String,
}

#[derive(Debug, Clone)]
struct RepositoryResponsibilityContract {
    workflow_path: String,
    workflow_name: String,
    binding_kind: String,
    track_key: Option<String>,
    source_binding: String,
    source_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityMapDrift {
    project_id: i64,
    repository_id: i64,
    repository: String,
    workflow_path: String,
    workflow_name: String,
    drift_type: String,
    repository_binding: String,
    watchtower_binding: Option<String>,
    expected_track_key: Option<String>,
    actual_track_key: Option<String>,
    source_path: String,
    reason: String,
    recommended_action: String,
    review_key: String,
    fingerprint: String,
    review_status: String,
    review_priority: String,
    review_age_bucket: String,
    review_age_hours: i64,
    review_event_count: i64,
    failed_attempt_count: i64,
    last_reviewed_at: Option<String>,
    first_seen_at: String,
    sla_status: String,
    sla_target_hours: Option<i64>,
    sla_remaining_hours: Option<i64>,
    escalation_level: String,
    escalation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityEscalationDelivery {
    id: i64,
    project_id: i64,
    repository_id: i64,
    repository: String,
    workflow_name: String,
    review_key: String,
    fingerprint: String,
    event_type: String,
    escalation_level: String,
    sla_status: String,
    reason: Option<String>,
    status: String,
    attempts: i64,
    last_error: Option<String>,
    first_attempt_at: String,
    last_attempt_at: String,
    emitted_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityResolutionPreviewInput {
    review_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveResponsibilityDriftInput {
    review_key: String,
    fingerprint: String,
    action: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeferResponsibilityDriftInput {
    review_key: String,
    fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityResolutionPreview {
    review_key: String,
    fingerprint: String,
    workflow_name: String,
    repository: String,
    repository_contract: String,
    watchtower_contract: Option<String>,
    action: String,
    changes: Vec<String>,
    invariants: Vec<String>,
    executable: bool,
    blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityResolutionResult {
    status: String,
    audit_id: i64,
    current_drift: Option<ResponsibilityMapDrift>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityResolutionAuditEntry {
    id: i64,
    project_id: i64,
    project_name: String,
    repository_id: i64,
    repository: String,
    workflow_name: String,
    review_key: String,
    drift_type: String,
    action: String,
    result: String,
    actor: String,
    created_at: String,
    requested_fingerprint: String,
    current_fingerprint: String,
    repository_contract: String,
    before_watchtower_contract: Option<String>,
    after_watchtower_contract: Option<String>,
    stale: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityMapSourceStatus {
    project_id: i64,
    repository_id: i64,
    repository: String,
    source_path: String,
    status: String,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    last_error: Option<String>,
    contract_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryResponsibilityMap {
    workflows: HashMap<String, RepositoryWorkflowResponsibility>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryWorkflowResponsibility {
    watchtower_track_binding: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubWorkflowsResponse {
    workflows: Vec<GithubWorkflowDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubWorkflowDefinition {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    id: i64,
    project_id: i64,
    name: String,
    track_key: String,
    long_ci_minutes: i64,
    active: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackInput {
    id: Option<i64>,
    project_id: i64,
    name: String,
    track_key: String,
    long_ci_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitoredRepository {
    id: i64,
    project_id: i64,
    repo: String,
    enabled: bool,
    running_count: i64,
    queued_count: i64,
    last_polled_at: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryInput {
    id: Option<i64>,
    project_id: i64,
    repo: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunSummary {
    id: i64,
    project_id: i64,
    repository_id: i64,
    repository: String,
    workflow_name: String,
    display_title: String,
    event: String,
    head_branch: Option<String>,
    head_sha: String,
    run_attempt: i64,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    created_at: String,
    run_started_at: Option<String>,
    updated_at: String,
    elapsed_seconds: i64,
    attribution_source: Option<String>,
    attribution_reason: Option<String>,
    confidence: Option<i64>,
    resolution_status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunAttributionEvidence {
    track_key: String,
    signal_type: String,
    score: i64,
    value: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReconciliationAuditEntry {
    id: i64,
    trigger: String,
    from_status: String,
    from_track_key: Option<String>,
    from_source: Option<String>,
    from_confidence: Option<i64>,
    to_status: String,
    to_track_key: Option<String>,
    to_source: Option<String>,
    to_confidence: Option<i64>,
    to_reason: Option<String>,
    previous_evidence: Vec<Evidence>,
    evidence: Vec<Evidence>,
    reconciled_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunAttributionDetail {
    run_id: i64,
    project_id: i64,
    repository_id: i64,
    repository: String,
    workflow_name: String,
    resolution_status: String,
    assigned_track_id: Option<i64>,
    assigned_track_name: Option<String>,
    assigned_track_key: Option<String>,
    source: Option<String>,
    reason: Option<String>,
    confidence: Option<i64>,
    manual: Option<bool>,
    last_resolution_attempt_at: Option<String>,
    project_rule_id: Option<i64>,
    project_rule_repository_id: Option<i64>,
    evidence: Vec<RunAttributionEvidence>,
    reconciliation_history: Vec<ReconciliationAuditEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardTrack {
    track: Track,
    health: String,
    elapsed_seconds: i64,
    average_duration_seconds: Option<i64>,
    runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    queue_congestion_threshold: i64,
    active_poll_seconds: i64,
    idle_poll_seconds: i64,
    auto_archive_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsibilityReviewPolicy {
    project_id: i64,
    p0_target_hours: i64,
    p1_target_hours: i64,
    p2_target_hours: i64,
    p0_due_soon_hours: i64,
    p1_due_soon_hours: i64,
    p2_due_soon_hours: i64,
    notify_warning: bool,
    notify_critical: bool,
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    running_count: usize,
    queued_count: usize,
    unassigned_count: usize,
    congestion_level: String,
    token_configured: bool,
    settings: Settings,
    projects: Vec<Project>,
    responsibility_review_policies: Vec<ResponsibilityReviewPolicy>,
    repositories: Vec<MonitoredRepository>,
    tracks: Vec<DashboardTrack>,
    project_workflow_rules: Vec<ProjectWorkflowRule>,
    dynamic_workflow_rules: Vec<DynamicWorkflowRule>,
    repository_scope_stats: Vec<RepositoryScopeStats>,
    producer_contract_stats: Vec<ProducerContractStats>,
    producer_contract_runs: Vec<ProducerContractRun>,
    responsibility_map_drifts: Vec<ResponsibilityMapDrift>,
    responsibility_map_sources: Vec<ResponsibilityMapSourceStatus>,
    responsibility_escalation_deliveries: Vec<ResponsibilityEscalationDelivery>,
    project_runs: Vec<WorkflowRunSummary>,
    unassigned_runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryScopeStats {
    repository_id: i64,
    project_id: i64,
    unassigned_count: i64,
    project_run_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProducerContractStats {
    repository_id: i64,
    project_id: i64,
    sampled_runs: i64,
    project_wide_runs: i64,
    explicit_runs: i64,
    run_name_runs: i64,
    pr_marker_runs: i64,
    commit_marker_runs: i64,
    branch_runs: i64,
    heuristic_runs: i64,
    manual_runs: i64,
    compatibility_runs: i64,
    unresolved_runs: i64,
    other_runs: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProducerContractRun {
    run: WorkflowRunSummary,
    bucket: String,
    contract_compliant: bool,
    responsibility_declared: bool,
    is_current_producer_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRunsResponse {
    workflow_runs: Vec<GithubRun>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRun {
    id: i64,
    workflow_id: i64,
    name: String,
    path: Option<String>,
    display_title: Option<String>,
    event: String,
    head_branch: Option<String>,
    head_sha: String,
    run_number: i64,
    #[serde(default = "default_attempt")]
    run_attempt: i64,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    created_at: String,
    run_started_at: Option<String>,
    updated_at: String,
    #[serde(default)]
    pull_requests: Vec<GithubPullRef>,
}

fn default_attempt() -> i64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
struct GithubPullRef {
    number: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubPull {
    number: i64,
    body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubCommitResponse {
    commit: GithubCommitInner,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubCommitInner {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Evidence {
    track_key: String,
    signal_type: String,
    score: i64,
    value: String,
}

#[derive(Debug, Clone)]
struct Resolution {
    status: String,
    track_id: Option<i64>,
    confidence: Option<i64>,
    source: Option<String>,
    reason: Option<String>,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Clone)]
struct Fingerprint {
    track_key: String,
    signal_type: String,
    pattern: String,
    weight: i64,
}

fn db(state: &AppState) -> Result<Connection> {
    Connection::open(&state.db_path).context("open sqlite")
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !names.iter().any(|name| name == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn init_db(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;

        CREATE TABLE IF NOT EXISTS tracks (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          repo TEXT NOT NULL,
          source_mode TEXT NOT NULL CHECK(source_mode IN ('branch','pr')),
          branch TEXT,
          pr_number INTEGER,
          workflow_filter TEXT,
          long_ci_minutes INTEGER NOT NULL CHECK(long_ci_minutes > 0),
          archived INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          CHECK (
            (source_mode='branch' AND branch IS NOT NULL AND pr_number IS NULL)
            OR
            (source_mode='pr' AND pr_number IS NOT NULL AND branch IS NULL)
          )
        );

        CREATE TABLE IF NOT EXISTS projects (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          project_key TEXT NOT NULL UNIQUE,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS watch_tracks (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
          name TEXT NOT NULL,
          track_key TEXT NOT NULL UNIQUE,
          long_ci_minutes INTEGER NOT NULL CHECK(long_ci_minutes > 0),
          active INTEGER NOT NULL DEFAULT 1,
          legacy_track_id INTEGER UNIQUE,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS monitored_repositories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          project_id INTEGER REFERENCES projects(id) ON DELETE RESTRICT,
          repo TEXT NOT NULL UNIQUE,
          enabled INTEGER NOT NULL DEFAULT 1,
          running_count INTEGER NOT NULL DEFAULT 0,
          queued_count INTEGER NOT NULL DEFAULT 0,
          last_polled_at TEXT,
          last_successful_poll_at TEXT,
          last_error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workflow_runs (
          run_id INTEGER PRIMARY KEY,
          repository_id INTEGER NOT NULL REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_id INTEGER NOT NULL,
          workflow_name TEXT NOT NULL,
          workflow_path TEXT,
          display_title TEXT NOT NULL,
          event TEXT NOT NULL,
          head_branch TEXT,
          head_sha TEXT NOT NULL,
          run_number INTEGER NOT NULL,
          run_attempt INTEGER NOT NULL,
          status TEXT NOT NULL,
          conclusion TEXT,
          html_url TEXT NOT NULL,
          created_at TEXT NOT NULL,
          run_started_at TEXT,
          updated_at TEXT NOT NULL,
          last_seen_at TEXT NOT NULL,
          last_resolution_attempt_at TEXT,
          resolution_status TEXT NOT NULL DEFAULT 'unassigned',
          ignored INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS run_attempts (
          run_id INTEGER NOT NULL,
          run_attempt INTEGER NOT NULL,
          status TEXT NOT NULL,
          conclusion TEXT,
          started_at TEXT,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(run_id, run_attempt)
        );

        CREATE TABLE IF NOT EXISTS run_assignments (
          run_id INTEGER PRIMARY KEY REFERENCES workflow_runs(run_id) ON DELETE CASCADE,
          track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
          confidence INTEGER NOT NULL,
          source TEXT NOT NULL,
          reason TEXT NOT NULL,
          manual INTEGER NOT NULL DEFAULT 0,
          assigned_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS run_evidence (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          run_id INTEGER NOT NULL REFERENCES workflow_runs(run_id) ON DELETE CASCADE,
          track_key TEXT NOT NULL,
          signal_type TEXT NOT NULL,
          score INTEGER NOT NULL,
          value TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS track_fingerprints (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
          signal_type TEXT NOT NULL,
          pattern TEXT NOT NULL,
          repository_id INTEGER REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          weight INTEGER NOT NULL CHECK(weight BETWEEN 1 AND 60),
          learned_from_run_id INTEGER,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          UNIQUE(track_id, signal_type, pattern, repository_id)
        );

        CREATE TABLE IF NOT EXISTS project_workflow_rules (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          repository_id INTEGER REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_name TEXT NOT NULL,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          UNIQUE(project_id, repository_id, workflow_name)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_project_workflow_rules_scope
          ON project_workflow_rules(project_id, COALESCE(repository_id,0), workflow_name);

        CREATE TABLE IF NOT EXISTS dynamic_workflow_rules (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          repository_id INTEGER REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_name TEXT NOT NULL,
          active INTEGER NOT NULL DEFAULT 1,
          protected INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_dynamic_workflow_rules_scope
          ON dynamic_workflow_rules(project_id, COALESCE(repository_id,0), workflow_name);

        CREATE TABLE IF NOT EXISTS repository_responsibility_contracts (
          repository_id INTEGER NOT NULL REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_path TEXT NOT NULL,
          workflow_name TEXT NOT NULL,
          binding_kind TEXT NOT NULL CHECK(binding_kind IN ('project-wide','dynamic','static','unknown')),
          track_key TEXT,
          source_binding TEXT NOT NULL,
          source_path TEXT NOT NULL,
          last_seen_at TEXT NOT NULL,
          PRIMARY KEY(repository_id, workflow_path)
        );

        CREATE INDEX IF NOT EXISTS idx_repository_responsibility_contracts_name
          ON repository_responsibility_contracts(repository_id, workflow_name);

        CREATE TABLE IF NOT EXISTS repository_responsibility_sources (
          repository_id INTEGER PRIMARY KEY REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          source_path TEXT NOT NULL,
          status TEXT NOT NULL CHECK(status IN ('synced','not_found','error')),
          last_attempt_at TEXT NOT NULL,
          last_success_at TEXT,
          last_error TEXT
        );

        CREATE TABLE IF NOT EXISTS responsibility_resolution_audit (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          review_key TEXT NOT NULL,
          fingerprint TEXT NOT NULL,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          repository_id INTEGER NOT NULL REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_name TEXT NOT NULL,
          drift_type TEXT NOT NULL,
          action TEXT NOT NULL,
          before_repository_binding TEXT NOT NULL,
          before_watchtower_binding TEXT,
          after_repository_binding TEXT,
          after_watchtower_binding TEXT,
          requested_fingerprint TEXT,
          current_fingerprint TEXT,
          expected_repository_binding TEXT,
          resulting_watchtower_binding TEXT,
          result TEXT NOT NULL CHECK(result IN ('resolved','deferred','blocked','stale_rejected','still_open','failed')),
          actor TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_responsibility_resolution_audit_review
          ON responsibility_resolution_audit(review_key, id DESC);

        CREATE TABLE IF NOT EXISTS responsibility_review_state (
          review_key TEXT NOT NULL,
          fingerprint TEXT NOT NULL,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          repository_id INTEGER NOT NULL REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_name TEXT NOT NULL,
          drift_type TEXT NOT NULL,
          first_seen_at TEXT NOT NULL,
          last_seen_at TEXT NOT NULL,
          PRIMARY KEY(review_key, fingerprint)
        );

        CREATE INDEX IF NOT EXISTS idx_responsibility_review_state_scope
          ON responsibility_review_state(project_id, repository_id, last_seen_at DESC);

        CREATE TABLE IF NOT EXISTS responsibility_review_policies (
          project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
          p0_target_hours INTEGER NOT NULL CHECK(p0_target_hours > 0),
          p1_target_hours INTEGER NOT NULL CHECK(p1_target_hours > 0),
          p2_target_hours INTEGER NOT NULL CHECK(p2_target_hours > 0),
          p0_due_soon_hours INTEGER NOT NULL CHECK(p0_due_soon_hours > 0),
          p1_due_soon_hours INTEGER NOT NULL CHECK(p1_due_soon_hours > 0),
          p2_due_soon_hours INTEGER NOT NULL CHECK(p2_due_soon_hours > 0),
          notify_warning INTEGER NOT NULL DEFAULT 1,
          notify_critical INTEGER NOT NULL DEFAULT 1,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS track_aliases (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          alias_key TEXT NOT NULL,
          track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          UNIQUE(project_id, alias_key)
        );

        CREATE TABLE IF NOT EXISTS assignment_migration_audit (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          migration_key TEXT NOT NULL,
          run_id INTEGER NOT NULL,
          repository_id INTEGER NOT NULL,
          repository_repo TEXT NOT NULL,
          from_repository_project_id INTEGER,
          to_repository_project_id INTEGER NOT NULL,
          track_id INTEGER NOT NULL,
          track_project_id INTEGER NOT NULL,
          track_key TEXT NOT NULL,
          track_name TEXT NOT NULL,
          confidence INTEGER NOT NULL,
          source TEXT NOT NULL,
          reason TEXT NOT NULL,
          manual INTEGER NOT NULL,
          assigned_at TEXT NOT NULL,
          action TEXT NOT NULL,
          migrated_at TEXT NOT NULL,
          UNIQUE(migration_key, run_id, track_id)
        );

        CREATE TABLE IF NOT EXISTS resolution_reconciliation_audit (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          run_id INTEGER NOT NULL REFERENCES workflow_runs(run_id) ON DELETE CASCADE,
          repository_id INTEGER NOT NULL,
          trigger TEXT NOT NULL,
          from_status TEXT NOT NULL,
          from_track_key TEXT,
          from_source TEXT,
          from_confidence INTEGER,
          to_status TEXT NOT NULL,
          to_track_key TEXT,
          to_source TEXT,
          to_confidence INTEGER,
          to_reason TEXT,
          previous_evidence_json TEXT NOT NULL,
          evidence_json TEXT NOT NULL,
          reconciled_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_resolution_reconciliation_run
          ON resolution_reconciliation_audit(run_id, reconciled_at DESC);

        CREATE TABLE IF NOT EXISTS notifications_v2 (
          track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
          run_id INTEGER NOT NULL,
          run_attempt INTEGER NOT NULL,
          event_type TEXT NOT NULL,
          notified_at TEXT NOT NULL,
          PRIMARY KEY(track_id, run_id, run_attempt, event_type)
        );

        CREATE TABLE IF NOT EXISTS responsibility_escalation_deliveries (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          review_key TEXT NOT NULL,
          fingerprint TEXT NOT NULL,
          project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
          repository_id INTEGER NOT NULL REFERENCES monitored_repositories(id) ON DELETE CASCADE,
          workflow_name TEXT NOT NULL,
          event_type TEXT NOT NULL CHECK(event_type IN ('warning','critical')),
          escalation_level TEXT NOT NULL,
          sla_status TEXT NOT NULL,
          reason TEXT,
          status TEXT NOT NULL CHECK(status IN ('pending','emitted','failed')),
          attempts INTEGER NOT NULL DEFAULT 0,
          last_error TEXT,
          first_attempt_at TEXT NOT NULL,
          last_attempt_at TEXT NOT NULL,
          emitted_at TEXT,
          UNIQUE(review_key, fingerprint, event_type)
        );

        CREATE INDEX IF NOT EXISTS idx_responsibility_escalation_delivery_scope
          ON responsibility_escalation_deliveries(project_id, repository_id, last_attempt_at DESC);

        CREATE TABLE IF NOT EXISTS app_settings (
          id INTEGER PRIMARY KEY CHECK(id=1),
          queue_congestion_threshold INTEGER NOT NULL,
          active_poll_seconds INTEGER NOT NULL,
          idle_poll_seconds INTEGER NOT NULL,
          auto_archive_completed INTEGER NOT NULL,
          queue_congested INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )?;
    ensure_column(&conn, "watch_tracks", "project_id", "INTEGER")?;
    ensure_column(&conn, "monitored_repositories", "project_id", "INTEGER")?;
    ensure_column(&conn, "workflow_runs", "last_resolution_attempt_at", "TEXT")?;
    ensure_column(
        &conn,
        "dynamic_workflow_rules",
        "protected",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &conn,
        "responsibility_resolution_audit",
        "requested_fingerprint",
        "TEXT",
    )?;
    ensure_column(
        &conn,
        "responsibility_resolution_audit",
        "current_fingerprint",
        "TEXT",
    )?;
    ensure_column(
        &conn,
        "responsibility_resolution_audit",
        "expected_repository_binding",
        "TEXT",
    )?;
    ensure_column(
        &conn,
        "responsibility_resolution_audit",
        "resulting_watchtower_binding",
        "TEXT",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO app_settings(id, queue_congestion_threshold, active_poll_seconds, idle_poll_seconds, auto_archive_completed, queue_congested) VALUES(1,?,?,?,?,0)",
        params![DEFAULT_QUEUE_THRESHOLD, DEFAULT_ACTIVE_POLL_SECONDS, DEFAULT_IDLE_POLL_SECONDS, 0],
    )?;
    migrate_legacy(&conn)?;
    migrate_project_scope(&conn)?;
    migrate_track_key_scope(&conn)?;
    migrate_track_alias_scope(&conn)?;
    seed_myeongha_aliases(&conn)?;
    seed_bejewely_project_scope(&conn)?;
    reconcile_alias_assignments(&conn)?;
    Ok(())
}

fn legacy_track_key(name: &str, id: i64) -> String {
    let lower = name.to_lowercase();
    if lower.contains("프론트") || lower.contains("frontend") {
        "frontend-integration".into()
    } else if lower.contains("관상 연구") || lower.contains("face research") {
        "face-research".into()
    } else if lower.contains("관상") || lower.contains("face reading") {
        "face-reading".into()
    } else if lower.contains("사주") || lower.contains("saju") {
        "saju".into()
    } else if lower.contains("운영") || lower.contains("ops") {
        "ops".into()
    } else if lower.contains("결제") || lower.contains("commerce") {
        "product-commerce".into()
    } else if lower.contains("파이프라인") || lower.contains("reliability") {
        "pipeline-reliability".into()
    } else {
        format!("legacy-{id}")
    }
}

fn migrate_legacy(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id,name,repo,long_ci_minutes,created_at,updated_at FROM tracks ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, name, repo, long_ci, created_at, updated_at) = row?;
        let key = legacy_track_key(&name, id);
        conn.execute(
            "INSERT OR IGNORE INTO watch_tracks(name,track_key,long_ci_minutes,active,legacy_track_id,created_at,updated_at) VALUES(?,?,?,1,?,?,?)",
            params![name, key, long_ci, id, created_at, updated_at],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO monitored_repositories(repo,enabled,created_at,updated_at) VALUES(?,1,?,?)",
            params![repo, now, now],
        )?;
    }
    Ok(())
}

fn migrate_track_key_scope(conn: &Connection) -> Result<()> {
    let table_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='watch_tracks'",
        [],
        |row| row.get(0),
    )?;
    let legacy_global_unique = table_sql
        .to_ascii_lowercase()
        .replace('\n', " ")
        .replace('\r', " ")
        .replace('\t', " ")
        .contains("track_key text not null unique");
    if !legacy_global_unique {
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_watch_tracks_project_key ON watch_tracks(project_id,track_key)",
            [],
        )?;
        return Ok(());
    }

    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         CREATE TABLE watch_tracks_v03 (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
           name TEXT NOT NULL,
           track_key TEXT NOT NULL,
           long_ci_minutes INTEGER NOT NULL,
           active INTEGER NOT NULL DEFAULT 1,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL,
           UNIQUE(project_id, track_key)
         );
         INSERT INTO watch_tracks_v03(id,project_id,name,track_key,long_ci_minutes,active,created_at,updated_at)
           SELECT id,project_id,name,track_key,long_ci_minutes,active,created_at,updated_at FROM watch_tracks;
         DROP TABLE watch_tracks;
         ALTER TABLE watch_tracks_v03 RENAME TO watch_tracks;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_watch_tracks_project_key ON watch_tracks(project_id,track_key);
         PRAGMA foreign_keys=ON;"
    )?;
    Ok(())
}

fn invalidate_cross_project_assignments(
    conn: &Connection,
    repository_id: i64,
    from_repository_project_id: Option<i64>,
    to_repository_project_id: i64,
    migration_key: &str,
) -> Result<()> {
    let migrated_at = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO assignment_migration_audit(
           migration_key,run_id,repository_id,repository_repo,
           from_repository_project_id,to_repository_project_id,
           track_id,track_project_id,track_key,track_name,
           confidence,source,reason,manual,assigned_at,action,migrated_at
         )
         SELECT ?,wr.run_id,wr.repository_id,mr.repo,?,?,
                ra.track_id,wt.project_id,wt.track_key,wt.name,
                ra.confidence,ra.source,ra.reason,ra.manual,ra.assigned_at,
                CASE
                  WHEN ra.manual=1 THEN 'manual-invalidated-for-project-move'
                  ELSE 'automatic-invalidated-for-project-move'
                END,
                ?
         FROM run_assignments ra
         JOIN workflow_runs wr ON wr.run_id=ra.run_id
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN watch_tracks wt ON wt.id=ra.track_id
         WHERE wr.repository_id=?
           AND wt.project_id<>?",
        params![
            migration_key,
            from_repository_project_id,
            to_repository_project_id,
            migrated_at,
            repository_id,
            to_repository_project_id
        ],
    )?;

    conn.execute(
        "DELETE FROM run_assignments
         WHERE run_id IN (
           SELECT wr.run_id
           FROM workflow_runs wr
           WHERE wr.repository_id=?
         )
         AND track_id IN (
           SELECT id FROM watch_tracks WHERE project_id<>?
         )",
        params![repository_id, to_repository_project_id],
    )?;

    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='assigned'
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id
           )",
        params![repository_id],
    )?;

    Ok(())
}

fn seed_bejewely_project_scope(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();

    tx.execute(
        "INSERT OR IGNORE INTO projects(name,project_key,active,created_at,updated_at)
         VALUES('비주얼리','visualy',1,?,?)",
        params![now, now],
    )?;
    let project_id: i64 = tx.query_row(
        "SELECT id FROM projects WHERE project_key='visualy' LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE projects SET name='비주얼리',active=1,updated_at=? WHERE id=?",
        params![now, project_id],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO monitored_repositories(
           project_id,repo,enabled,running_count,queued_count,created_at,updated_at
         ) VALUES(?,'gycha0109-beep/K_beauty',1,0,0,?,?)",
        params![project_id, now, now],
    )?;
    let (repository_id, previous_repository_project_id): (i64, Option<i64>) = tx.query_row(
        "SELECT id,project_id
         FROM monitored_repositories
         WHERE repo='gycha0109-beep/K_beauty'
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    invalidate_cross_project_assignments(
        &tx,
        repository_id,
        previous_repository_project_id,
        project_id,
        "visualy-project-scope-v1",
    )?;

    tx.execute(
        "UPDATE monitored_repositories SET project_id=?,updated_at=?
         WHERE id=?",
        params![project_id, now, repository_id],
    )?;

    for (name, track_key, long_ci_minutes) in [
        ("CI Watchtower / CI 운영 정리", "ops", 8_i64),
        ("데이터 정렬 & AI", "taxonomy-ai", 8_i64),
        ("신규 상품 신뢰도 운영 파이프라인", "trust", 8_i64),
        ("Face Lab 연구", "face-research", 8_i64),
        ("Premium Full Report", "full-report", 8_i64),
        ("Mobile", "mobile", 8_i64),
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO watch_tracks(
               project_id,name,track_key,long_ci_minutes,active,created_at,updated_at
             ) VALUES(?,?,?,?,1,?,?)",
            params![project_id, name, track_key, long_ci_minutes, now, now],
        )?;
    }

    for workflow_name in ["BEJEWELY Current Main Health", "PIE Prospective Shadow"] {
        tx.execute(
            "INSERT OR IGNORE INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,NULL,?,1,?)",
            params![project_id, workflow_name, now],
        )?;
    }

    // Visualy's repository-owned responsibility map is authoritative for producer class.
    // Security/Supply Chain were previously seeded as Project-wide, but are now
    // explicitly shared-dynamic producers. Remove the stale classification and
    // return affected non-manual runs to fail-closed reconciliation.
    tx.execute(
        "DELETE FROM project_workflow_rules
         WHERE project_id=?
           AND workflow_name IN ('BEJEWELY Security Boundary','BEJEWELY Supply Chain Security')",
        params![project_id],
    )?;
    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='project'
           AND workflow_name IN ('BEJEWELY Security Boundary','BEJEWELY Supply Chain Security')
           AND NOT EXISTS(
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        params![repository_id],
    )?;

    for workflow_name in [
        "Admin - Access Foundation",
        "Admin - Product Current Main Integration",
        "Product Offer Runtime - DATA-OFFER17 Controlled RPC Diagnostic",
        "Product Data Pipeline - Hwahae Provenance Non-Main PR Guard",
        "Product Data Pipeline - Identity Key Repair Confirm",
        "Product Data Pipeline - Product Offers",
        "Product Data Pipeline - Source Bindings",
        "BEJEWELY Security Boundary",
        "Recommendation Admission - G3A PF Authority Read",
        "BEJEWELY Supply Chain Security",
        "BEJEWELY AI Provider Runtime",
        "BEJEWELY Database Integration Authority",
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,?,1,1,?)",
            params![project_id, repository_id, workflow_name, now],
        )?;
        tx.execute(
            "UPDATE dynamic_workflow_rules
             SET active=1,protected=1
             WHERE project_id=? AND repository_id=? AND workflow_name=?",
            params![project_id, repository_id, workflow_name],
        )?;
    }

    tx.execute(
        "DELETE FROM run_assignments
         WHERE manual=0
           AND run_id IN (
             SELECT wr.run_id
             FROM workflow_runs wr
             JOIN project_workflow_rules pwr
               ON pwr.project_id=?
              AND pwr.active=1
              AND pwr.workflow_name=wr.workflow_name
              AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
             WHERE wr.repository_id=?
           )",
        params![project_id, repository_id],
    )?;

    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='project'
         WHERE repository_id=?
           AND ignored=0
           AND EXISTS (
             SELECT 1
             FROM project_workflow_rules pwr
             WHERE pwr.project_id=?
               AND pwr.active=1
               AND pwr.workflow_name=workflow_runs.workflow_name
               AND (pwr.repository_id IS NULL OR pwr.repository_id=workflow_runs.repository_id)
           )
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        params![repository_id, project_id],
    )?;

    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='assigned'
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id
           )",
        params![repository_id],
    )?;

    tx.commit()?;
    Ok(())
}

fn migrate_project_scope(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();

    let has_myeongha: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM monitored_repositories WHERE repo LIKE '%/MyeongHa' OR repo LIKE '%/Saju')",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;

    let project_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
    if project_count == 0 {
        let (name, key) = if has_myeongha {
            ("명하", "myeongha")
        } else {
            ("기본 프로젝트", "default")
        };
        conn.execute(
            "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES(?,?,1,?,?)",
            params![name, key, now, now],
        )?;
    }

    let default_project_id: i64 = if has_myeongha {
        conn.query_row(
            "SELECT id FROM projects WHERE project_key='myeongha' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .or_else(|_| {
            conn.execute(
                "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES('명하','myeongha',1,?,?)",
                params![now, now],
            )?;
            Ok::<i64, rusqlite::Error>(conn.last_insert_rowid())
        })?
    } else {
        conn.query_row(
            "SELECT id FROM projects WHERE active=1 ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?
    };

    conn.execute(
        "UPDATE monitored_repositories SET project_id=? WHERE project_id IS NULL",
        params![default_project_id],
    )?;
    conn.execute(
        "UPDATE watch_tracks SET project_id=? WHERE project_id IS NULL",
        params![default_project_id],
    )?;

    let commerce_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM watch_tracks WHERE track_key='commerce' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let product_commerce_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM watch_tracks WHERE track_key='product-commerce')",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    if let Some(id) = commerce_id {
        if !product_commerce_exists {
            conn.execute(
                "UPDATE watch_tracks SET track_key='product-commerce',updated_at=? WHERE id=?",
                params![now, id],
            )?;
        }
    }

    if has_myeongha {
        let myeongha_project_id: i64 = conn.query_row(
            "SELECT project_id FROM monitored_repositories WHERE repo LIKE '%/MyeongHa' OR repo LIKE '%/Saju' ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        for workflow_name in [
            "CI",
            "Governance",
            "Web PR Domain Gates",
            "PIE Prospective Shadow",
        ] {
            conn.execute(
                "INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
                 VALUES(?,NULL,?,1,?)",
                params![myeongha_project_id, workflow_name, now],
            )?;
        }

        let myeongha_repository_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/MyeongHa'
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(repository_id) = myeongha_repository_id {
            for workflow_name in [
                "DB Content Reading Suite",
                "DB Runtime Authority Suite",
                "DB PostgreSQL 17 Authority Suite",
                "Supabase Production",
                "Web Browser Smoke",
                "Web Auth Browser Regression",
            ] {
                conn.execute(
                    "INSERT OR IGNORE INTO project_workflow_rules(
                       project_id,repository_id,workflow_name,active,created_at
                     ) VALUES(?,?,?,1,?)",
                    params![myeongha_project_id, repository_id, workflow_name, now],
                )?;
            }

            conn.execute(
                "INSERT OR IGNORE INTO dynamic_workflow_rules(
                   project_id,repository_id,workflow_name,active,protected,created_at
                 ) VALUES(?,?,'Production Records Current-Subject Smoke',1,1,?)",
                params![myeongha_project_id, repository_id, now],
            )?;
            conn.execute(
                "UPDATE dynamic_workflow_rules
                 SET active=1,protected=1
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='Production Records Current-Subject Smoke'",
                params![myeongha_project_id, repository_id],
            )?;
        }

        let saju_repository_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/Saju'
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(repository_id) = saju_repository_id {
            conn.execute(
                "INSERT OR IGNORE INTO dynamic_workflow_rules(
                   project_id,repository_id,workflow_name,active,protected,created_at
                 ) VALUES(?,?,'MESH6J Manual Browser Capture Surface CI',1,1,?)",
                params![myeongha_project_id, repository_id, now],
            )?;
            conn.execute(
                "UPDATE dynamic_workflow_rules
                 SET active=1,protected=1
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='MESH6J Manual Browser Capture Surface CI'",
                params![myeongha_project_id, repository_id],
            )?;
        }
    }

    conn.execute(
        "DELETE FROM run_assignments
         WHERE manual=0 AND run_id IN (
           SELECT wr.run_id
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           JOIN project_workflow_rules pwr
             ON pwr.project_id=mr.project_id
            AND pwr.active=1
            AND pwr.workflow_name=wr.workflow_name
            AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
         )",
        [],
    )?;
    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='project'
         WHERE ignored=0
           AND run_id IN (
             SELECT wr.run_id
             FROM workflow_runs wr
             JOIN monitored_repositories mr ON mr.id=wr.repository_id
             JOIN project_workflow_rules pwr
               ON pwr.project_id=mr.project_id
              AND pwr.active=1
              AND pwr.workflow_name=wr.workflow_name
              AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
           )
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        [],
    )?;

    Ok(())
}

fn migrate_track_alias_scope(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(track_aliases)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_project_id = columns.iter().any(|name| name == "project_id");

    if !has_project_id {
        let orphan_count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM track_aliases ta
             LEFT JOIN watch_tracks wt ON wt.id=ta.track_id
             WHERE wt.id IS NULL",
            [],
            |row| row.get(0),
        )?;
        if orphan_count > 0 {
            return Err(anyhow!(
                "Track alias migration을 중단했습니다. 대상 Track이 없는 alias가 {}개 있습니다.",
                orphan_count
            ));
        }

        conn.execute_batch(
            "CREATE TABLE track_aliases_v04 (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               alias_key TEXT NOT NULL,
               track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
               active INTEGER NOT NULL DEFAULT 1,
               created_at TEXT NOT NULL,
               UNIQUE(project_id, alias_key)
             );
             INSERT INTO track_aliases_v04(project_id,alias_key,track_id,active,created_at)
               SELECT wt.project_id,ta.alias_key,ta.track_id,ta.active,ta.created_at
               FROM track_aliases ta
               JOIN watch_tracks wt ON wt.id=ta.track_id;
             DROP TABLE track_aliases;
             ALTER TABLE track_aliases_v04 RENAME TO track_aliases;",
        )?;
    }

    conn.execute(
        "UPDATE track_aliases
         SET project_id=(SELECT wt.project_id FROM watch_tracks wt WHERE wt.id=track_aliases.track_id)
         WHERE EXISTS(
           SELECT 1 FROM watch_tracks wt
           WHERE wt.id=track_aliases.track_id AND wt.project_id<>track_aliases.project_id
         )",
        [],
    )?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_track_aliases_project_key
         ON track_aliases(project_id,alias_key)",
        [],
    )?;
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS trg_track_aliases_project_insert
         BEFORE INSERT ON track_aliases
         FOR EACH ROW
         WHEN NOT EXISTS(
           SELECT 1 FROM watch_tracks wt
           WHERE wt.id=NEW.track_id AND wt.project_id=NEW.project_id
         )
         BEGIN
           SELECT RAISE(ABORT, 'track alias project mismatch');
         END;

         CREATE TRIGGER IF NOT EXISTS trg_track_aliases_project_update
         BEFORE UPDATE OF project_id,track_id ON track_aliases
         FOR EACH ROW
         WHEN NOT EXISTS(
           SELECT 1 FROM watch_tracks wt
           WHERE wt.id=NEW.track_id AND wt.project_id=NEW.project_id
         )
         BEGIN
           SELECT RAISE(ABORT, 'track alias project mismatch');
         END;",
    )?;
    Ok(())
}

fn seed_myeongha_aliases(conn: &Connection) -> Result<()> {
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM projects WHERE project_key='myeongha' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let now = Utc::now().to_rfc3339();

    for (alias_key, track_key) in [
        ("privacy-recovery", "ops"),
        ("commerce", "product-commerce"),
    ] {
        let track_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM watch_tracks
                 WHERE project_id=? AND track_key=? AND active=1
                 LIMIT 1",
                params![project_id, track_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(track_id) = track_id {
            conn.execute(
                "INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at)
                 VALUES(?,?,?,1,?)
                 ON CONFLICT(project_id,alias_key)
                 DO UPDATE SET track_id=excluded.track_id,active=1",
                params![project_id, alias_key, track_id, now],
            )?;
        }
    }
    Ok(())
}

fn reconcile_alias_assignments(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO run_assignments(run_id,track_id,confidence,source,reason,manual,assigned_at)
         SELECT wr.run_id,ta.track_id,MAX(re.score),'track_alias',
                '과거 Track Key alias 자동 귀속',0,?
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN run_evidence re ON re.run_id=wr.run_id
         JOIN track_aliases ta
           ON ta.project_id=mr.project_id
          AND ta.alias_key=re.track_key
          AND ta.active=1
         JOIN watch_tracks wt
           ON wt.id=ta.track_id
          AND wt.project_id=mr.project_id
          AND wt.active=1
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         WHERE wr.ignored=0
           AND wr.resolution_status IN ('unassigned','conflict')
           AND re.score>=90
           AND ra.run_id IS NULL
         GROUP BY wr.run_id,ta.track_id",
        params![now],
    )?;
    conn.execute(
        "UPDATE workflow_runs SET resolution_status='assigned'
         WHERE ignored=0 AND resolution_status IN ('unassigned','conflict')
           AND EXISTS(SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id)",
        [],
    )?;
    Ok(())
}

fn keyring_entry() -> Result<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| anyhow!(e.to_string()))
}

fn github_token() -> Result<String> {
    keyring_entry()?
        .get_password()
        .map_err(|_| anyhow!("GitHub PAT이 설정되지 않았습니다."))
}

fn token_configured() -> bool {
    keyring_entry()
        .and_then(|e| e.get_password().map_err(|err| anyhow!(err.to_string())))
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn github_client(token: &str) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        header::HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    Ok(Client::builder()
        .default_headers(headers)
        .user_agent("ci-watchtower/0.3.28")
        .timeout(Duration::from_secs(20))
        .build()?)
}

fn validate_repo(repo: &str) -> Result<()> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(anyhow!("Repository는 owner/name 형식이어야 합니다."));
    }
    if !owner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(anyhow!("Repository 이름에 허용되지 않은 문자가 있습니다."));
    }
    Ok(())
}

fn validate_track_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 64 {
        return Err(anyhow!("Track Key는 1~64자여야 합니다."));
    }
    if key.starts_with('-') || key.ends_with('-') {
        return Err(anyhow!(
            "Track Key는 하이픈으로 시작하거나 끝날 수 없습니다."
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow!(
            "Track Key는 영문 소문자, 숫자, 하이픈만 사용할 수 있습니다."
        ));
    }
    Ok(())
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

fn elapsed_seconds(
    status: &str,
    created_at: &str,
    started_at: Option<&str>,
    updated_at: &str,
    now: DateTime<Utc>,
) -> i64 {
    let start = started_at
        .and_then(parse_time)
        .or_else(|| parse_time(created_at));
    let end = if status == "completed" {
        parse_time(updated_at).unwrap_or(now)
    } else {
        now
    };
    start.map(|s| (end - s).num_seconds().max(0)).unwrap_or(0)
}

fn normalize_evidence_track_key(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "taxonomy&ai" => "taxonomy-ai".into(),
        key => key.into(),
    }
}

fn normalize_repository_binding(binding: &str) -> (String, Option<String>) {
    let binding = binding.trim();
    if binding == "unassigned-by-design" {
        return ("project-wide".into(), None);
    }
    if binding == "dynamic-by-run" {
        return ("dynamic".into(), None);
    }
    if let Some(track_key) = binding.strip_prefix("static:") {
        let track_key = normalize_evidence_track_key(track_key.trim());
        if validate_track_key(&track_key).is_ok() {
            return ("static".into(), Some(track_key));
        }
    }
    ("unknown".into(), None)
}

fn extract_marker(text: &str) -> Option<String> {
    let start = text.find("[WT:")? + 4;
    let tail = &text[start..];
    let end = tail.find(']')?;
    let key = normalize_evidence_track_key(&tail[..end]);
    validate_track_key(&key).ok()?;
    Some(key)
}

fn extract_track_trailer(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Watchtower-Track:") {
            let key = normalize_evidence_track_key(value);
            if validate_track_key(&key).is_ok() {
                return Some(key);
            }
        }
    }
    None
}

fn branch_has_key(branch: &str, key: &str) -> bool {
    let normalized_branch = branch.to_lowercase().replace("taxonomy&ai", "taxonomy-ai");
    normalized_branch == key
        || normalized_branch.split('/').any(|segment| segment == key)
        || normalized_branch.starts_with(&format!("{key}/"))
        || normalized_branch.ends_with(&format!("/{key}"))
}

fn load_settings(conn: &Connection) -> Result<Settings> {
    conn.query_row(
        "SELECT queue_congestion_threshold,active_poll_seconds,idle_poll_seconds,auto_archive_completed FROM app_settings WHERE id=1",
        [],
        |row| {
            Ok(Settings {
                queue_congestion_threshold: row.get(0)?,
                active_poll_seconds: row.get(1)?,
                idle_poll_seconds: row.get(2)?,
                auto_archive_completed: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .map_err(Into::into)
}

fn list_projects(conn: &Connection, active_only: bool) -> Result<Vec<Project>> {
    let sql = if active_only {
        "SELECT id,name,project_key,active,created_at,updated_at FROM projects WHERE active=1 ORDER BY id"
    } else {
        "SELECT id,name,project_key,active,created_at,updated_at FROM projects ORDER BY id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            project_key: row.get(2)?,
            active: row.get::<_, i64>(3)? != 0,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_tracks(conn: &Connection, active_only: bool) -> Result<Vec<Track>> {
    let sql = if active_only {
        "SELECT id,project_id,name,track_key,long_ci_minutes,active,created_at,updated_at FROM watch_tracks WHERE active=1 ORDER BY id DESC"
    } else {
        "SELECT id,project_id,name,track_key,long_ci_minutes,active,created_at,updated_at FROM watch_tracks ORDER BY id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Track {
            id: row.get(0)?,
            project_id: row.get(1)?,
            name: row.get(2)?,
            track_key: row.get(3)?,
            long_ci_minutes: row.get(4)?,
            active: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_repositories(conn: &Connection, enabled_only: bool) -> Result<Vec<MonitoredRepository>> {
    let sql = if enabled_only {
        "SELECT id,project_id,repo,enabled,running_count,queued_count,last_polled_at,last_error FROM monitored_repositories WHERE enabled=1 ORDER BY repo"
    } else {
        "SELECT id,project_id,repo,enabled,running_count,queued_count,last_polled_at,last_error FROM monitored_repositories ORDER BY repo"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(MonitoredRepository {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            running_count: row.get(4)?,
            queued_count: row.get(5)?,
            last_polled_at: row.get(6)?,
            last_error: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_project_workflow_rules(conn: &Connection) -> Result<Vec<ProjectWorkflowRule>> {
    let mut stmt = conn.prepare(
        "SELECT id,project_id,repository_id,workflow_name,active
         FROM project_workflow_rules WHERE active=1 ORDER BY project_id,workflow_name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ProjectWorkflowRule {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repository_id: row.get(2)?,
            workflow_name: row.get(3)?,
            active: row.get::<_, i64>(4)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_dynamic_workflow_rules(conn: &Connection) -> Result<Vec<DynamicWorkflowRule>> {
    let mut stmt = conn.prepare(
        "SELECT id,project_id,repository_id,workflow_name,active,protected
         FROM dynamic_workflow_rules WHERE active=1 ORDER BY project_id,workflow_name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(DynamicWorkflowRule {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repository_id: row.get(2)?,
            workflow_name: row.get(3)?,
            active: row.get::<_, i64>(4)? != 0,
            protected: row.get::<_, i64>(5)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn dynamic_rule_matches(
    rules: &[DynamicWorkflowRule],
    project_id: i64,
    repository_id: i64,
    workflow_name: &str,
) -> bool {
    rules.iter().any(|rule| {
        rule.active
            && rule.project_id == project_id
            && rule.workflow_name == workflow_name
            && (rule.repository_id.is_none() || rule.repository_id == Some(repository_id))
    })
}

fn update_repository_responsibility_source(
    conn: &Connection,
    repository_id: i64,
    status: &str,
    now: &str,
    error: Option<&str>,
) -> Result<()> {
    if !matches!(status, "synced" | "not_found" | "error") {
        return Err(anyhow!(
            "지원하지 않는 responsibility map source status: {status}"
        ));
    }
    let successful_at = (status == "synced").then_some(now);
    conn.execute(
        "INSERT INTO repository_responsibility_sources(
           repository_id,source_path,status,last_attempt_at,last_success_at,last_error
         ) VALUES(?,?,?,?,?,?)
         ON CONFLICT(repository_id) DO UPDATE SET
           source_path=excluded.source_path,
           status=excluded.status,
           last_attempt_at=excluded.last_attempt_at,
           last_success_at=CASE
             WHEN excluded.status='synced' THEN excluded.last_attempt_at
             ELSE repository_responsibility_sources.last_success_at
           END,
           last_error=excluded.last_error",
        params![
            repository_id,
            RESPONSIBILITY_MAP_PATH,
            status,
            now,
            successful_at,
            error
        ],
    )?;
    Ok(())
}

fn responsibility_map_source_statuses(
    conn: &Connection,
) -> Result<Vec<ResponsibilityMapSourceStatus>> {
    let mut stmt = conn.prepare(
        "SELECT mr.project_id,mr.id,mr.repo,
                COALESCE(rrs.source_path,?),
                COALESCE(rrs.status,'pending'),
                rrs.last_attempt_at,rrs.last_success_at,rrs.last_error,
                (
                  SELECT COUNT(*)
                  FROM repository_responsibility_contracts rrc
                  WHERE rrc.repository_id=mr.id
                )
         FROM monitored_repositories mr
         LEFT JOIN repository_responsibility_sources rrs
           ON rrs.repository_id=mr.id
         WHERE mr.enabled=1
         ORDER BY mr.project_id,mr.repo",
    )?;
    let rows = stmt.query_map(params![RESPONSIBILITY_MAP_PATH], |row| {
        Ok(ResponsibilityMapSourceStatus {
            project_id: row.get(0)?,
            repository_id: row.get(1)?,
            repository: row.get(2)?,
            source_path: row.get(3)?,
            status: row.get(4)?,
            last_attempt_at: row.get(5)?,
            last_success_at: row.get(6)?,
            last_error: row.get(7)?,
            contract_count: row.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn replace_repository_responsibility_contracts(
    conn: &Connection,
    repository_id: i64,
    contracts: &[RepositoryResponsibilityContract],
    now: &str,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM repository_responsibility_contracts WHERE repository_id=?",
        params![repository_id],
    )?;
    for contract in contracts {
        tx.execute(
            "INSERT INTO repository_responsibility_contracts(
               repository_id,workflow_path,workflow_name,binding_kind,track_key,
               source_binding,source_path,last_seen_at
             ) VALUES(?,?,?,?,?,?,?,?)",
            params![
                repository_id,
                contract.workflow_path,
                contract.workflow_name,
                contract.binding_kind,
                contract.track_key,
                contract.source_binding,
                contract.source_path,
                now
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn repository_has_workflow_run(
    conn: &Connection,
    repository_id: i64,
    workflow_name: &str,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM workflow_runs
           WHERE repository_id=? AND workflow_name=?
         )",
        params![repository_id, workflow_name],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )
    .map_err(Into::into)
}

fn latest_workflow_path(
    conn: &Connection,
    repository_id: i64,
    workflow_name: &str,
) -> Result<String> {
    let value: Option<Option<String>> = conn
        .query_row(
            "SELECT workflow_path FROM workflow_runs
             WHERE repository_id=? AND workflow_name=?
             ORDER BY updated_at DESC LIMIT 1",
            params![repository_id, workflow_name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.flatten().unwrap_or_default())
}

fn latest_workflow_run_title(
    conn: &Connection,
    repository_id: i64,
    workflow_path: &str,
    workflow_name: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT display_title FROM workflow_runs
             WHERE repository_id=?
               AND (workflow_path=? OR workflow_name=?)
             ORDER BY updated_at DESC LIMIT 1",
            params![repository_id, workflow_path, workflow_name],
            |row| row.get(0),
        )
        .optional()?)
}

fn stable_fingerprint(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn responsibility_review_key(
    project_id: i64,
    repository_id: i64,
    workflow_path: &str,
    workflow_name: &str,
    drift_type: &str,
) -> String {
    stable_fingerprint(&format!(
        "{project_id}|{repository_id}|{workflow_path}|{workflow_name}|{drift_type}"
    ))
}

fn responsibility_drift_fingerprint(drift: &ResponsibilityMapDrift) -> String {
    stable_fingerprint(&format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        drift.project_id,
        drift.repository_id,
        drift.workflow_path,
        drift.workflow_name,
        drift.drift_type,
        drift.repository_binding,
        drift.watchtower_binding.as_deref().unwrap_or(""),
        drift.expected_track_key.as_deref().unwrap_or(""),
        drift.actual_track_key.as_deref().unwrap_or(""),
        drift.source_path,
    ))
}

fn responsibility_review_status(
    conn: &Connection,
    review_key: &str,
    fingerprint: &str,
    recommended_action: &str,
) -> Result<String> {
    if matches!(
        recommended_action,
        "review_track_registry_or_map"
            | "review_producer_run_name"
            | "review_repository_map_binding"
            | "review_remove_conflicting_responsibility_rule"
    ) {
        return Ok("blocked".into());
    }
    let latest: Option<(String, String)> = conn
        .query_row(
            "SELECT action,result FROM responsibility_resolution_audit
             WHERE review_key=? AND fingerprint=?
             ORDER BY id DESC LIMIT 1",
            params![review_key, fingerprint],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match latest
        .as_ref()
        .map(|(action, result)| (action.as_str(), result.as_str()))
    {
        Some(("reopen", _)) => "open",
        Some((_, "deferred")) => "deferred",
        Some((_, "failed" | "still_open" | "stale_rejected" | "blocked")) => "attention",
        _ => "open",
    }
    .into())
}

fn default_responsibility_review_policy(project_id: i64) -> ResponsibilityReviewPolicy {
    ResponsibilityReviewPolicy {
        project_id,
        p0_target_hours: DEFAULT_RESPONSIBILITY_P0_TARGET_HOURS,
        p1_target_hours: DEFAULT_RESPONSIBILITY_P1_TARGET_HOURS,
        p2_target_hours: DEFAULT_RESPONSIBILITY_P2_TARGET_HOURS,
        p0_due_soon_hours: DEFAULT_RESPONSIBILITY_P0_DUE_SOON_HOURS,
        p1_due_soon_hours: DEFAULT_RESPONSIBILITY_P1_DUE_SOON_HOURS,
        p2_due_soon_hours: DEFAULT_RESPONSIBILITY_P2_DUE_SOON_HOURS,
        notify_warning: true,
        notify_critical: true,
        updated_at: None,
    }
}

fn responsibility_review_policy(
    conn: &Connection,
    project_id: i64,
) -> Result<ResponsibilityReviewPolicy> {
    Ok(conn
        .query_row(
            "SELECT project_id,p0_target_hours,p1_target_hours,p2_target_hours,
                    p0_due_soon_hours,p1_due_soon_hours,p2_due_soon_hours,
                    notify_warning,notify_critical,updated_at
             FROM responsibility_review_policies WHERE project_id=?",
            params![project_id],
            |row| {
                Ok(ResponsibilityReviewPolicy {
                    project_id: row.get(0)?,
                    p0_target_hours: row.get(1)?,
                    p1_target_hours: row.get(2)?,
                    p2_target_hours: row.get(3)?,
                    p0_due_soon_hours: row.get(4)?,
                    p1_due_soon_hours: row.get(5)?,
                    p2_due_soon_hours: row.get(6)?,
                    notify_warning: row.get::<_, i64>(7)? != 0,
                    notify_critical: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            },
        )
        .optional()?
        .unwrap_or_else(|| default_responsibility_review_policy(project_id)))
}

fn list_responsibility_review_policies(
    conn: &Connection,
    projects: &[Project],
) -> Result<Vec<ResponsibilityReviewPolicy>> {
    projects
        .iter()
        .map(|project| responsibility_review_policy(conn, project.id))
        .collect()
}

fn responsibility_review_priority(
    policy: &ResponsibilityReviewPolicy,
    status: &str,
    age_hours: i64,
) -> String {
    match status {
        "attention" => "p0".into(),
        "open" if age_hours >= policy.p2_target_hours => "p1".into(),
        "open" => "p2".into(),
        "deferred" => "p3".into(),
        "blocked" => "blocked".into(),
        _ => "p2".into(),
    }
}

fn responsibility_review_priority_rank(priority: &str) -> i64 {
    match priority {
        "p0" => 0,
        "p1" => 1,
        "p2" => 2,
        "p3" => 3,
        "blocked" => 4,
        _ => 5,
    }
}

fn responsibility_review_age_bucket(age_hours: i64) -> String {
    if age_hours >= 72 {
        "overdue".into()
    } else if age_hours >= 24 {
        "aging".into()
    } else {
        "fresh".into()
    }
}

fn responsibility_review_sla(
    policy: &ResponsibilityReviewPolicy,
    priority: &str,
    age_hours: i64,
) -> (String, Option<i64>, Option<i64>, String, Option<String>) {
    let (target_hours, due_soon_hours) = match priority {
        "p0" => (Some(policy.p0_target_hours), policy.p0_due_soon_hours),
        "p1" => (Some(policy.p1_target_hours), policy.p1_due_soon_hours),
        "p2" => (Some(policy.p2_target_hours), policy.p2_due_soon_hours),
        "p3" | "blocked" => (None, 0),
        _ => (None, 0),
    };
    let Some(target_hours) = target_hours else {
        return ("exempt".into(), None, None, "none".into(), None);
    };
    let remaining = target_hours - age_hours;
    if remaining <= 0 {
        let reason = match priority {
            "p0" => format!(
                "P0 attention drift exceeded the {}h review SLA by {}h.",
                target_hours, -remaining
            ),
            "p1" => format!(
                "P1 aging drift exceeded the {}h review SLA by {}h.",
                target_hours, -remaining
            ),
            _ => format!("Review SLA exceeded by {}h.", -remaining),
        };
        return (
            "breached".into(),
            Some(target_hours),
            Some(remaining),
            if matches!(priority, "p0" | "p1") {
                "critical".into()
            } else {
                "none".into()
            },
            Some(reason),
        );
    }
    let status = if remaining <= due_soon_hours {
        "due_soon"
    } else {
        "within_sla"
    };
    let escalation_level = if matches!(priority, "p0" | "p1") {
        "warning"
    } else {
        "none"
    };
    let reason = match priority {
        "p0" => Some(format!(
            "P0 attention drift has {}h remaining before the {}h review SLA.",
            remaining, target_hours
        )),
        "p1" => Some(format!(
            "P1 aging drift has {}h remaining before the {}h review SLA.",
            remaining, target_hours
        )),
        _ => None,
    };
    (
        status.into(),
        Some(target_hours),
        Some(remaining),
        escalation_level.into(),
        reason,
    )
}

fn responsibility_escalation_rank(level: &str) -> i64 {
    match level {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn populate_responsibility_review_operations(
    conn: &Connection,
    drift: &mut ResponsibilityMapDrift,
) -> Result<()> {
    let now = Utc::now();
    let now_text = now.to_rfc3339();
    conn.execute(
        "INSERT INTO responsibility_review_state(
           review_key,fingerprint,project_id,repository_id,workflow_name,drift_type,first_seen_at,last_seen_at
         ) VALUES(?,?,?,?,?,?,?,?)
         ON CONFLICT(review_key,fingerprint) DO UPDATE SET
           project_id=excluded.project_id,
           repository_id=excluded.repository_id,
           workflow_name=excluded.workflow_name,
           drift_type=excluded.drift_type,
           last_seen_at=excluded.last_seen_at",
        params![
            drift.review_key,
            drift.fingerprint,
            drift.project_id,
            drift.repository_id,
            drift.workflow_name,
            drift.drift_type,
            now_text,
            now_text,
        ],
    )?;

    let first_seen_at: String = conn.query_row(
        "SELECT first_seen_at FROM responsibility_review_state
         WHERE review_key=? AND fingerprint=?",
        params![drift.review_key, drift.fingerprint],
        |row| row.get(0),
    )?;
    let first_seen = DateTime::parse_from_rfc3339(&first_seen_at)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or(now);
    let age_hours = now.signed_duration_since(first_seen).num_hours().max(0);

    let (event_count, failed_attempt_count, last_reviewed_at): (i64, i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE
                      WHEN result IN ('failed','stale_rejected') THEN 1
                      WHEN result='still_open' AND action<>'reopen' THEN 1
                      ELSE 0 END),0),
                    MAX(created_at)
             FROM responsibility_resolution_audit
             WHERE review_key=? AND fingerprint=?",
            params![drift.review_key, drift.fingerprint],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

    drift.first_seen_at = first_seen_at;
    drift.review_age_hours = age_hours;
    drift.review_age_bucket = responsibility_review_age_bucket(age_hours);
    drift.review_event_count = event_count;
    drift.failed_attempt_count = failed_attempt_count;
    drift.last_reviewed_at = last_reviewed_at;
    let policy = responsibility_review_policy(conn, drift.project_id)?;
    drift.review_priority =
        responsibility_review_priority(&policy, &drift.review_status, age_hours);
    let (sla_status, sla_target_hours, sla_remaining_hours, escalation_level, escalation_reason) =
        responsibility_review_sla(&policy, &drift.review_priority, age_hours);
    drift.sla_status = sla_status;
    drift.sla_target_hours = sla_target_hours;
    drift.sla_remaining_hours = sla_remaining_hours;
    drift.escalation_level = escalation_level;
    drift.escalation_reason = escalation_reason;
    Ok(())
}

fn finalize_responsibility_drifts(
    conn: &Connection,
    drifts: &mut [ResponsibilityMapDrift],
) -> Result<()> {
    for drift in drifts {
        drift.review_key = responsibility_review_key(
            drift.project_id,
            drift.repository_id,
            &drift.workflow_path,
            &drift.workflow_name,
            &drift.drift_type,
        );
        drift.fingerprint = responsibility_drift_fingerprint(drift);
        drift.review_status = responsibility_review_status(
            conn,
            &drift.review_key,
            &drift.fingerprint,
            &drift.recommended_action,
        )?;
        populate_responsibility_review_operations(conn, drift)?;
    }
    Ok(())
}

fn responsibility_map_drifts(conn: &Connection) -> Result<Vec<ResponsibilityMapDrift>> {
    let project_rules = list_project_workflow_rules(conn)?;
    let dynamic_rules = list_dynamic_workflow_rules(conn)?;
    let mut stmt = conn.prepare(
        "SELECT rrc.repository_id,mr.project_id,mr.repo,
                rrc.workflow_path,rrc.workflow_name,rrc.binding_kind,
                rrc.track_key,rrc.source_binding,rrc.source_path
         FROM repository_responsibility_contracts rrc
         JOIN monitored_repositories mr ON mr.id=rrc.repository_id
         ORDER BY rrc.repository_id,rrc.workflow_path",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            RepositoryResponsibilityContract {
                workflow_path: row.get(3)?,
                workflow_name: row.get(4)?,
                binding_kind: row.get(5)?,
                track_key: row.get(6)?,
                source_binding: row.get(7)?,
                source_path: row.get(8)?,
            },
        ))
    })?;
    let contracts = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut drifts = Vec::new();
    let mut repository_contract_names: HashMap<i64, HashSet<String>> = HashMap::new();
    let mut repository_meta: HashMap<i64, (i64, String)> = HashMap::new();

    for (repository_id, project_id, repository, contract) in &contracts {
        repository_contract_names
            .entry(*repository_id)
            .or_default()
            .insert(contract.workflow_name.clone());
        repository_meta
            .entry(*repository_id)
            .or_insert((*project_id, repository.clone()));

        let project_declared = project_rule_matches(
            &project_rules,
            *project_id,
            *repository_id,
            &contract.workflow_name,
        );
        let dynamic_declared = dynamic_rule_matches(
            &dynamic_rules,
            *project_id,
            *repository_id,
            &contract.workflow_name,
        );
        let latest_run_title = latest_workflow_run_title(
            conn,
            *repository_id,
            &contract.workflow_path,
            &contract.workflow_name,
        )?;
        let actual_track_key = latest_run_title.as_deref().and_then(extract_marker);
        let actual_binding = if project_declared && dynamic_declared {
            Some("project-wide + dynamic".to_string())
        } else if project_declared {
            Some("project-wide".to_string())
        } else if dynamic_declared {
            Some("dynamic".to_string())
        } else {
            actual_track_key.as_ref().map(|key| format!("static:{key}"))
        };

        let drift_review: Option<(&str, String, &str)> = match contract.binding_kind.as_str() {
            "project-wide" => {
                if dynamic_declared {
                    Some((
                        "responsibility_kind_mismatch",
                        format!(
                            "Repository map declares {}, but WatchTower currently declares {}.",
                            contract.source_binding,
                            actual_binding.as_deref().unwrap_or("missing")
                        ),
                        "review_reclassify_project_wide",
                    ))
                } else if !project_declared {
                    Some((
                        "missing_in_watchtower",
                        format!(
                            "Repository map declares {}, but WatchTower has no matching active Project-wide rule.",
                            contract.source_binding
                        ),
                        "review_add_project_wide_rule",
                    ))
                } else {
                    None
                }
            }
            "dynamic" => {
                if project_declared {
                    Some((
                        "responsibility_kind_mismatch",
                        format!(
                            "Repository map declares {}, but WatchTower currently declares {}.",
                            contract.source_binding,
                            actual_binding.as_deref().unwrap_or("missing")
                        ),
                        "review_reclassify_dynamic",
                    ))
                } else if !dynamic_declared {
                    Some((
                        "missing_in_watchtower",
                        format!(
                            "Repository map declares {}, but WatchTower has no matching active Dynamic rule.",
                            contract.source_binding
                        ),
                        "review_add_dynamic_rule",
                    ))
                } else {
                    None
                }
            }
            "static" => {
                if project_declared || dynamic_declared {
                    Some((
                        "responsibility_kind_mismatch",
                        format!(
                            "Repository map declares {}, but WatchTower currently declares {} instead of a static Track binding.",
                            contract.source_binding,
                            actual_binding.as_deref().unwrap_or("missing")
                        ),
                        "review_remove_conflicting_responsibility_rule",
                    ))
                } else {
                    let expected = contract.track_key.as_deref();
                    let track_exists = expected
                        .map(|key| {
                            conn.query_row(
                                "SELECT EXISTS(
                                   SELECT 1 FROM watch_tracks
                                   WHERE project_id=? AND track_key=? AND active=1
                                 )",
                                params![project_id, key],
                                |row| Ok(row.get::<_, i64>(0)? != 0),
                            )
                            .unwrap_or(false)
                        })
                        .unwrap_or(false);
                    if !track_exists {
                        Some((
                            "track_binding_mismatch",
                            format!(
                                "Repository map expects {}, but the referenced canonical Track is not active in this Project.",
                                contract.source_binding
                            ),
                            "review_track_registry_or_map",
                        ))
                    } else if latest_run_title.is_some() && actual_track_key.as_deref() != expected
                    {
                        Some((
                            "track_binding_mismatch",
                            format!(
                                "Repository map expects {}, but the latest producer run-name evidence resolves to {}.",
                                contract.source_binding,
                                actual_track_key.as_deref().unwrap_or("no [WT:*] marker")
                            ),
                            "review_producer_run_name",
                        ))
                    } else {
                        None
                    }
                }
            }
            _ => Some((
                "responsibility_kind_mismatch",
                format!(
                    "Repository map contains unsupported responsibility binding {}.",
                    contract.source_binding
                ),
                "review_repository_map_binding",
            )),
        };

        if let Some((drift_type, reason, recommended_action)) = drift_review {
            drifts.push(ResponsibilityMapDrift {
                project_id: *project_id,
                repository_id: *repository_id,
                repository: repository.clone(),
                workflow_path: contract.workflow_path.clone(),
                workflow_name: contract.workflow_name.clone(),
                drift_type: drift_type.into(),
                repository_binding: contract.source_binding.clone(),
                watchtower_binding: actual_binding,
                expected_track_key: contract.track_key.clone(),
                actual_track_key,
                source_path: contract.source_path.clone(),
                reason,
                recommended_action: recommended_action.into(),
                review_key: String::new(),
                fingerprint: String::new(),
                review_status: "open".into(),
                review_priority: "p2".into(),
                review_age_bucket: "fresh".into(),
                review_age_hours: 0,
                review_event_count: 0,
                failed_attempt_count: 0,
                last_reviewed_at: None,
                first_seen_at: String::new(),
                sla_status: "within_sla".into(),
                sla_target_hours: Some(72),
                sla_remaining_hours: Some(72),
                escalation_level: "none".into(),
                escalation_reason: None,
            });
        }
    }

    let mut stale_seen: HashSet<(i64, String, String)> = HashSet::new();
    for (repository_id, contract_names) in &repository_contract_names {
        let Some((project_id, repository)) = repository_meta.get(repository_id) else {
            continue;
        };
        for rule in project_rules.iter().filter(|rule| {
            rule.project_id == *project_id
                && (rule.repository_id.is_none() || rule.repository_id == Some(*repository_id))
        }) {
            if !contract_names.contains(&rule.workflow_name)
                && repository_has_workflow_run(conn, *repository_id, &rule.workflow_name)?
                && stale_seen.insert((
                    *repository_id,
                    rule.workflow_name.clone(),
                    "project-wide".into(),
                ))
            {
                drifts.push(ResponsibilityMapDrift {
                    project_id: *project_id,
                    repository_id: *repository_id,
                    repository: repository.clone(),
                    workflow_path: latest_workflow_path(conn, *repository_id, &rule.workflow_name)?,
                    workflow_name: rule.workflow_name.clone(),
                    drift_type: "stale_in_watchtower".into(),
                    repository_binding: "missing-from-repository-map".into(),
                    watchtower_binding: Some("project-wide".into()),
                    expected_track_key: None,
                    actual_track_key: None,
                    source_path: RESPONSIBILITY_MAP_PATH.into(),
                    reason: "WatchTower still declares this workflow as Project-wide, but the current repository responsibility map no longer contains the producer.".into(),
                    recommended_action: "review_remove_or_confirm_stale_rule".into(),
                    review_key: String::new(),
                    fingerprint: String::new(),
                    review_status: "open".into(),
                    review_priority: "p2".into(),
                    review_age_bucket: "fresh".into(),
                    review_age_hours: 0,
                    review_event_count: 0,
                    failed_attempt_count: 0,
                    last_reviewed_at: None,
                    first_seen_at: String::new(),
                sla_status: "within_sla".into(),
                sla_target_hours: Some(72),
                sla_remaining_hours: Some(72),
                escalation_level: "none".into(),
                escalation_reason: None,
                });
            }
        }
        for rule in dynamic_rules.iter().filter(|rule| {
            rule.project_id == *project_id
                && (rule.repository_id.is_none() || rule.repository_id == Some(*repository_id))
        }) {
            if !contract_names.contains(&rule.workflow_name)
                && repository_has_workflow_run(conn, *repository_id, &rule.workflow_name)?
                && stale_seen.insert((*repository_id, rule.workflow_name.clone(), "dynamic".into()))
            {
                drifts.push(ResponsibilityMapDrift {
                    project_id: *project_id,
                    repository_id: *repository_id,
                    repository: repository.clone(),
                    workflow_path: latest_workflow_path(conn, *repository_id, &rule.workflow_name)?,
                    workflow_name: rule.workflow_name.clone(),
                    drift_type: "stale_in_watchtower".into(),
                    repository_binding: "missing-from-repository-map".into(),
                    watchtower_binding: Some("dynamic".into()),
                    expected_track_key: None,
                    actual_track_key: None,
                    source_path: RESPONSIBILITY_MAP_PATH.into(),
                    reason: "WatchTower still declares this workflow as Dynamic, but the current repository responsibility map no longer contains the producer.".into(),
                    recommended_action: "review_remove_or_confirm_stale_rule".into(),
                    review_key: String::new(),
                    fingerprint: String::new(),
                    review_status: "open".into(),
                    review_priority: "p2".into(),
                    review_age_bucket: "fresh".into(),
                    review_age_hours: 0,
                    review_event_count: 0,
                    failed_attempt_count: 0,
                    last_reviewed_at: None,
                    first_seen_at: String::new(),
                sla_status: "within_sla".into(),
                sla_target_hours: Some(72),
                sla_remaining_hours: Some(72),
                escalation_level: "none".into(),
                escalation_reason: None,
                });
            }
        }
    }

    finalize_responsibility_drifts(conn, &mut drifts)?;
    drifts.sort_by(|a, b| {
        responsibility_escalation_rank(&a.escalation_level)
            .cmp(&responsibility_escalation_rank(&b.escalation_level))
            .then_with(|| {
                responsibility_review_priority_rank(&a.review_priority)
                    .cmp(&responsibility_review_priority_rank(&b.review_priority))
            })
            .then_with(|| b.review_age_hours.cmp(&a.review_age_hours))
            .then_with(|| b.failed_attempt_count.cmp(&a.failed_attempt_count))
            .then_with(|| a.repository.cmp(&b.repository))
            .then_with(|| a.workflow_name.cmp(&b.workflow_name))
            .then_with(|| a.drift_type.cmp(&b.drift_type))
    });
    Ok(drifts)
}

fn load_run_attribution_detail(conn: &Connection, run_id: i64) -> Result<RunAttributionDetail> {
    let (
        run_id,
        project_id,
        repository_id,
        repository,
        workflow_name,
        resolution_status,
        assigned_track_id,
        assigned_track_name,
        assigned_track_key,
        assignment_source,
        assignment_reason,
        assignment_confidence,
        assignment_manual,
        last_resolution_attempt_at,
    ): (
        i64,
        i64,
        i64,
        String,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT wr.run_id,mr.project_id,mr.id,mr.repo,wr.workflow_name,wr.resolution_status,
                    wt.id,wt.name,wt.track_key,ra.source,ra.reason,ra.confidence,ra.manual,
                    wr.last_resolution_attempt_at
             FROM workflow_runs wr
             JOIN monitored_repositories mr ON mr.id=wr.repository_id
             LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
             LEFT JOIN watch_tracks wt ON wt.id=ra.track_id
             WHERE wr.run_id=?",
            params![run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("Run을 찾지 못했습니다."))?;

    let mut evidence_stmt = conn.prepare(
        "SELECT track_key,signal_type,score,value,created_at
         FROM run_evidence
         WHERE run_id=?
         ORDER BY score DESC,id ASC",
    )?;
    let evidence = evidence_stmt
        .query_map(params![run_id], |row| {
            Ok(RunAttributionEvidence {
                track_key: row.get(0)?,
                signal_type: row.get(1)?,
                score: row.get(2)?,
                value: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let project_rule: Option<(i64, Option<i64>)> = if resolution_status == "project" {
        conn.query_row(
            "SELECT id,repository_id
             FROM project_workflow_rules
             WHERE project_id=?
               AND workflow_name=?
               AND active=1
               AND (repository_id IS NULL OR repository_id=?)
             ORDER BY CASE WHEN repository_id=? THEN 0 ELSE 1 END,id
             LIMIT 1",
            params![project_id, workflow_name, repository_id, repository_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    } else {
        None
    };

    let (project_rule_id, project_rule_repository_id) = project_rule
        .map(|(id, repository_id)| (Some(id), repository_id))
        .unwrap_or((None, None));

    let (source, reason, confidence) = if resolution_status == "project" {
        (
            Some("project_workflow".into()),
            Some("프로젝트 공용 CI 규칙".into()),
            Some(100),
        )
    } else {
        (assignment_source, assignment_reason, assignment_confidence)
    };

    let has_reconciliation_audit: bool = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master
           WHERE type='table' AND name='resolution_reconciliation_audit'
         )",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    let reconciliation_history = if has_reconciliation_audit {
        let mut history_stmt = conn.prepare(
            "SELECT id,trigger,from_status,from_track_key,from_source,from_confidence,
                    to_status,to_track_key,to_source,to_confidence,to_reason,
                    previous_evidence_json,evidence_json,reconciled_at
             FROM resolution_reconciliation_audit
             WHERE run_id=?
             ORDER BY id DESC
             LIMIT 20",
        )?;
        let rows = history_stmt.query_map(params![run_id], |row| {
            let previous_json: String = row.get(11)?;
            let evidence_json: String = row.get(12)?;
            Ok(ReconciliationAuditEntry {
                id: row.get(0)?,
                trigger: row.get(1)?,
                from_status: row.get(2)?,
                from_track_key: row.get(3)?,
                from_source: row.get(4)?,
                from_confidence: row.get(5)?,
                to_status: row.get(6)?,
                to_track_key: row.get(7)?,
                to_source: row.get(8)?,
                to_confidence: row.get(9)?,
                to_reason: row.get(10)?,
                previous_evidence: serde_json::from_str(&previous_json).unwrap_or_default(),
                evidence: serde_json::from_str(&evidence_json).unwrap_or_default(),
                reconciled_at: row.get(13)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    Ok(RunAttributionDetail {
        run_id,
        project_id,
        repository_id,
        repository,
        workflow_name,
        resolution_status,
        assigned_track_id,
        assigned_track_name,
        assigned_track_key,
        source,
        reason,
        confidence,
        manual: assignment_manual.map(|value| value != 0),
        last_resolution_attempt_at,
        project_rule_id,
        project_rule_repository_id,
        evidence,
        reconciliation_history,
    })
}

fn run_summary_from_row(
    row: &rusqlite::Row<'_>,
    now: DateTime<Utc>,
) -> rusqlite::Result<WorkflowRunSummary> {
    let status: String = row.get(10)?;
    let created_at: String = row.get(14)?;
    let run_started_at: Option<String> = row.get(15)?;
    let updated_at: String = row.get(16)?;
    Ok(WorkflowRunSummary {
        id: row.get(0)?,
        project_id: row.get(1)?,
        repository_id: row.get(2)?,
        repository: row.get(3)?,
        workflow_name: row.get(4)?,
        display_title: row.get(5)?,
        event: row.get(6)?,
        head_branch: row.get(7)?,
        head_sha: row.get(8)?,
        run_attempt: row.get(9)?,
        status: status.clone(),
        conclusion: row.get(11)?,
        html_url: row.get(12)?,
        resolution_status: row.get(13)?,
        created_at: created_at.clone(),
        run_started_at: run_started_at.clone(),
        updated_at: updated_at.clone(),
        elapsed_seconds: elapsed_seconds(
            &status,
            &created_at,
            run_started_at.as_deref(),
            &updated_at,
            now,
        ),
        attribution_source: row.get(17)?,
        attribution_reason: row.get(18)?,
        confidence: row.get(19)?,
    })
}

fn runs_for_track(conn: &Connection, track_id: i64, limit: i64) -> Result<Vec<WorkflowRunSummary>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "WITH ranked AS (
           SELECT wr.run_id,mr.project_id,mr.id AS repository_id,mr.repo,
                  wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,
                  wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,
                  wr.created_at,wr.run_started_at,wr.updated_at,
                  ra.source,ra.reason,ra.confidence,
                  ROW_NUMBER() OVER (
                    PARTITION BY wr.repository_id
                    ORDER BY wr.created_at DESC
                  ) AS repository_rank
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           JOIN run_assignments ra ON ra.run_id=wr.run_id
           WHERE ra.track_id=? AND wr.ignored=0
         )
         SELECT run_id,project_id,repository_id,repo,workflow_name,display_title,event,
                head_branch,head_sha,run_attempt,status,conclusion,html_url,resolution_status,
                created_at,run_started_at,updated_at,source,reason,confidence
         FROM ranked
         WHERE repository_rank<=?
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![track_id, limit], |row| {
        run_summary_from_row(row, now)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn project_runs_for_repository(
    conn: &Connection,
    repository_id: i64,
    limit: i64,
) -> Result<Vec<WorkflowRunSummary>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT wr.run_id,mr.project_id,mr.id,mr.repo,wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,wr.created_at,wr.run_started_at,wr.updated_at,
                'project_workflow' AS attribution_source,
                '프로젝트 공용 CI 규칙' AS attribution_reason,
                100 AS confidence
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         WHERE wr.repository_id=?
           AND wr.resolution_status='project'
           AND wr.ignored=0
         ORDER BY wr.created_at DESC LIMIT ?",
    )?;
    let rows = stmt.query_map(params![repository_id, limit], |row| {
        run_summary_from_row(row, now)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn unassigned_runs_for_repository(
    conn: &Connection,
    repository_id: i64,
    limit: i64,
) -> Result<Vec<WorkflowRunSummary>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT wr.run_id,mr.project_id,mr.id,mr.repo,wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,wr.created_at,wr.run_started_at,wr.updated_at,
                CASE WHEN wr.resolution_status='conflict' THEN 'explicit_conflict'
                     ELSE (SELECT re.signal_type FROM run_evidence re WHERE re.run_id=wr.run_id ORDER BY re.score DESC LIMIT 1) END,
                (SELECT 'Track Key 후보: ' || group_concat(track_key, ', ') FROM (SELECT DISTINCT re.track_key track_key FROM run_evidence re WHERE re.run_id=wr.run_id AND re.score>=90)),
                (SELECT MAX(re.score) FROM run_evidence re WHERE re.run_id=wr.run_id)
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         WHERE wr.repository_id=?
           AND ra.run_id IS NULL
           AND wr.ignored=0
           AND COALESCE(wr.workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
           AND wr.resolution_status IN ('unassigned','conflict')
         ORDER BY CASE WHEN wr.status='completed' THEN 1 ELSE 0 END, wr.created_at DESC
         LIMIT ?",
    )?;
    let rows = stmt.query_map(params![repository_id, limit], |row| {
        run_summary_from_row(row, now)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn repository_scope_stats(conn: &Connection) -> Result<Vec<RepositoryScopeStats>> {
    let mut stmt = conn.prepare(
        "SELECT mr.id,mr.project_id,
                SUM(CASE
                      WHEN wr.run_id IS NOT NULL
                       AND wr.ignored=0
                       AND COALESCE(wr.workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
                       AND wr.resolution_status IN ('unassigned','conflict')
                       AND ra.run_id IS NULL
                      THEN 1 ELSE 0
                    END) AS unassigned_count,
                SUM(CASE
                      WHEN wr.run_id IS NOT NULL
                       AND wr.ignored=0
                       AND wr.resolution_status='project'
                      THEN 1 ELSE 0
                    END) AS project_run_count
         FROM monitored_repositories mr
         LEFT JOIN workflow_runs wr ON wr.repository_id=mr.id
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         GROUP BY mr.id,mr.project_id
         ORDER BY mr.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(RepositoryScopeStats {
            repository_id: row.get(0)?,
            project_id: row.get(1)?,
            unassigned_count: row.get(2)?,
            project_run_count: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn producer_contract_stats(
    conn: &Connection,
    sample_per_repository: i64,
) -> Result<Vec<ProducerContractStats>> {
    let mut stmt = conn.prepare(
        "WITH recent AS (
           SELECT wr.run_id,mr.id AS repository_id,mr.project_id,
                  wr.resolution_status,ra.source,COALESCE(ra.manual,0) AS manual,
                  ROW_NUMBER() OVER (
                    PARTITION BY wr.repository_id
                    ORDER BY wr.created_at DESC,wr.run_id DESC
                  ) AS repository_rank
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
           WHERE wr.ignored=0
             AND COALESCE(wr.workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
         )
         SELECT repository_id,project_id,
                COUNT(*) AS sampled_runs,
                SUM(CASE WHEN resolution_status='project' THEN 1 ELSE 0 END) AS project_wide_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0
                          AND source IN ('run_name','pr_marker','commit_marker','branch')
                         THEN 1 ELSE 0 END) AS explicit_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='run_name' THEN 1 ELSE 0 END) AS run_name_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='pr_marker' THEN 1 ELSE 0 END) AS pr_marker_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='commit_marker' THEN 1 ELSE 0 END) AS commit_marker_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='branch' THEN 1 ELSE 0 END) AS branch_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='inference' THEN 1 ELSE 0 END) AS heuristic_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=1 THEN 1 ELSE 0 END) AS manual_runs,
                SUM(CASE WHEN resolution_status='assigned' AND manual=0 AND source='track_alias' THEN 1 ELSE 0 END) AS compatibility_runs,
                SUM(CASE WHEN resolution_status IN ('unassigned','conflict') THEN 1 ELSE 0 END) AS unresolved_runs,
                SUM(CASE WHEN resolution_status NOT IN ('project','unassigned','conflict')
                           AND NOT (
                             resolution_status='assigned'
                             AND (
                               manual=1
                               OR source IN ('run_name','pr_marker','commit_marker','branch','inference','track_alias')
                             )
                           )
                         THEN 1 ELSE 0 END) AS other_runs
         FROM recent
         WHERE repository_rank<=?
         GROUP BY repository_id,project_id
         ORDER BY repository_id",
    )?;
    let rows = stmt.query_map(params![sample_per_repository.max(1)], |row| {
        Ok(ProducerContractStats {
            repository_id: row.get(0)?,
            project_id: row.get(1)?,
            sampled_runs: row.get(2)?,
            project_wide_runs: row.get(3)?,
            explicit_runs: row.get(4)?,
            run_name_runs: row.get(5)?,
            pr_marker_runs: row.get(6)?,
            commit_marker_runs: row.get(7)?,
            branch_runs: row.get(8)?,
            heuristic_runs: row.get(9)?,
            manual_runs: row.get(10)?,
            compatibility_runs: row.get(11)?,
            unresolved_runs: row.get(12)?,
            other_runs: row.get(13)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn producer_contract_runs(
    conn: &Connection,
    sample_per_repository: i64,
) -> Result<Vec<ProducerContractRun>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "WITH recent AS (
           SELECT wr.run_id,wr.workflow_id,mr.project_id,mr.id AS repository_id,mr.repo,
                  wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,
                  wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,
                  wr.created_at,wr.run_started_at,wr.updated_at,
                  CASE
                    WHEN wr.resolution_status='project' THEN 'project_workflow'
                    WHEN wr.resolution_status='conflict' THEN 'explicit_conflict'
                    ELSE ra.source
                  END AS attribution_source,
                  CASE
                    WHEN wr.resolution_status='project' THEN '프로젝트 공용 CI 규칙'
                    WHEN ra.reason IS NOT NULL THEN ra.reason
                    ELSE (
                      SELECT 'Track Key 후보: ' || group_concat(track_key, ', ')
                      FROM (
                        SELECT DISTINCT re.track_key track_key
                        FROM run_evidence re
                        WHERE re.run_id=wr.run_id AND re.score>=90
                      )
                    )
                  END AS attribution_reason,
                  CASE
                    WHEN wr.resolution_status='project' THEN 100
                    WHEN ra.confidence IS NOT NULL THEN ra.confidence
                    ELSE (SELECT MAX(re.score) FROM run_evidence re WHERE re.run_id=wr.run_id)
                  END AS confidence,
                  COALESCE(ra.manual,0) AS manual,
                  ROW_NUMBER() OVER (
                    PARTITION BY wr.repository_id
                    ORDER BY wr.created_at DESC,wr.run_id DESC
                  ) AS repository_rank,
                  ROW_NUMBER() OVER (
                    PARTITION BY wr.repository_id,wr.workflow_id
                    ORDER BY wr.created_at DESC,wr.run_id DESC
                  ) AS workflow_rank
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
           WHERE wr.ignored=0
             AND COALESCE(wr.workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
         )
         SELECT run_id,project_id,repository_id,repo,workflow_name,display_title,event,
                head_branch,head_sha,run_attempt,status,conclusion,html_url,resolution_status,
                created_at,run_started_at,updated_at,attribution_source,attribution_reason,confidence,
                CASE
                  WHEN resolution_status='project' THEN 'project'
                  WHEN resolution_status='assigned' AND manual=0
                       AND attribution_source IN ('run_name','pr_marker','commit_marker','branch')
                    THEN attribution_source
                  WHEN resolution_status='assigned' AND manual=0 AND attribution_source='inference'
                    THEN 'inference'
                  WHEN resolution_status='assigned' AND manual=1 THEN 'manual'
                  WHEN resolution_status='assigned' AND manual=0 AND attribution_source='track_alias'
                    THEN 'track_alias'
                  WHEN resolution_status IN ('unassigned','conflict') THEN resolution_status
                  ELSE 'other'
                END AS bucket,
                CASE
                  WHEN resolution_status='project' THEN 1
                  WHEN resolution_status='assigned' AND manual=0
                       AND attribution_source IN ('run_name','pr_marker','commit_marker','branch')
                    THEN 1
                  ELSE 0
                END AS contract_compliant,
                CASE
                  WHEN resolution_status='project' THEN 1
                  WHEN resolution_status='assigned' AND manual=0 AND attribution_source='run_name' THEN 1
                  WHEN EXISTS(
                    SELECT 1 FROM dynamic_workflow_rules dwr
                    WHERE dwr.project_id=recent.project_id
                      AND dwr.active=1
                      AND dwr.workflow_name=recent.workflow_name
                      AND (dwr.repository_id IS NULL OR dwr.repository_id=recent.repository_id)
                  ) THEN 1
                  ELSE 0
                END AS responsibility_declared,
                CASE WHEN workflow_rank=1 THEN 1 ELSE 0 END AS is_current_producer_run
         FROM recent
         WHERE repository_rank<=?
         ORDER BY created_at DESC,run_id DESC",
    )?;
    let rows = stmt.query_map(params![sample_per_repository.max(1)], |row| {
        Ok(ProducerContractRun {
            run: run_summary_from_row(row, now)?,
            bucket: row.get(20)?,
            contract_compliant: row.get::<_, i64>(21)? != 0,
            responsibility_declared: row.get::<_, i64>(22)? != 0,
            is_current_producer_run: row.get::<_, i64>(23)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn track_health(runs: &[WorkflowRunSummary]) -> String {
    if runs.is_empty() {
        return "waiting".into();
    }
    if runs.iter().any(|r| r.status == "in_progress") {
        return "running".into();
    }
    if runs.iter().any(|r| {
        matches!(
            r.status.as_str(),
            "queued" | "requested" | "pending" | "waiting"
        )
    }) {
        return "queued".into();
    }
    let latest = &runs[0];
    if latest.status == "completed" {
        match latest.conclusion.as_deref() {
            Some("success") => "green".into(),
            Some(
                "failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure"
                | "stale",
            ) => "red".into(),
            _ => "completed_other".into(),
        }
    } else {
        "waiting".into()
    }
}

fn average_duration(conn: &Connection, track_id: i64) -> Result<Option<i64>> {
    let avg: Option<f64> = conn
        .query_row(
            "SELECT AVG(duration_seconds) FROM (
               SELECT CAST(strftime('%s',wr.updated_at)-strftime('%s',COALESCE(wr.run_started_at,wr.created_at)) AS INTEGER) duration_seconds
               FROM workflow_runs wr JOIN run_assignments ra ON ra.run_id=wr.run_id
               WHERE ra.track_id=? AND wr.status='completed'
               ORDER BY wr.updated_at DESC LIMIT 20
             )",
            params![track_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(avg.map(|v| v.max(0.0).round() as i64))
}

fn build_dashboard(state: &AppState) -> Result<Dashboard> {
    let conn = db(state)?;
    let settings = load_settings(&conn)?;
    let projects = list_projects(&conn, true)?;
    let responsibility_review_policies = list_responsibility_review_policies(&conn, &projects)?;
    let repositories = list_repositories(&conn, false)?;
    let tracks = list_tracks(&conn, true)?;
    let project_workflow_rules = list_project_workflow_rules(&conn)?;
    let dynamic_workflow_rules = list_dynamic_workflow_rules(&conn)?;
    let mut dashboard_tracks = Vec::with_capacity(tracks.len());
    for track in tracks {
        let runs = runs_for_track(&conn, track.id, 30)?;
        let health = track_health(&runs);
        let elapsed_seconds = runs
            .iter()
            .filter(|r| {
                matches!(
                    r.status.as_str(),
                    "in_progress" | "queued" | "requested" | "pending" | "waiting"
                )
            })
            .map(|r| r.elapsed_seconds)
            .max()
            .unwrap_or(0);
        dashboard_tracks.push(DashboardTrack {
            average_duration_seconds: average_duration(&conn, track.id)?,
            track,
            health,
            elapsed_seconds,
            runs,
        });
    }
    let running_count: i64 = repositories
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.running_count)
        .sum();
    let queued_count: i64 = repositories
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.queued_count)
        .sum();
    let repository_scope_stats = repository_scope_stats(&conn)?;
    let producer_contract_stats =
        producer_contract_stats(&conn, PRODUCER_CONTRACT_SAMPLE_PER_REPOSITORY)?;
    let producer_contract_runs =
        producer_contract_runs(&conn, PRODUCER_CONTRACT_SAMPLE_PER_REPOSITORY)?;
    let responsibility_map_drifts = responsibility_map_drifts(&conn)?;
    let responsibility_map_sources = responsibility_map_source_statuses(&conn)?;
    let responsibility_escalation_deliveries = responsibility_escalation_deliveries(&conn, 50)?;
    let mut project_runs = Vec::new();
    let mut unassigned_runs = Vec::new();
    for repository in &repositories {
        project_runs.extend(project_runs_for_repository(&conn, repository.id, 200)?);
        unassigned_runs.extend(unassigned_runs_for_repository(&conn, repository.id, 200)?);
    }
    project_runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    unassigned_runs.sort_by(|a, b| {
        let a_completed = a.status == "completed";
        let b_completed = b.status == "completed";
        a_completed
            .cmp(&b_completed)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
    let unassigned_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM workflow_runs wr
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         WHERE ra.run_id IS NULL
           AND wr.ignored=0
           AND COALESCE(wr.workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
           AND wr.resolution_status IN ('unassigned','conflict')",
        [],
        |row| row.get(0),
    )?;
    let congestion_level = if queued_count >= settings.queue_congestion_threshold {
        "congested"
    } else if queued_count >= (settings.queue_congestion_threshold.max(2) + 1) / 2 {
        "busy"
    } else {
        "safe"
    };
    Ok(Dashboard {
        running_count: running_count.max(0) as usize,
        queued_count: queued_count.max(0) as usize,
        unassigned_count: unassigned_count.max(0) as usize,
        congestion_level: congestion_level.into(),
        token_configured: token_configured(),
        settings,
        projects,
        responsibility_review_policies,
        repositories,
        tracks: dashboard_tracks,
        project_workflow_rules,
        dynamic_workflow_rules,
        repository_scope_stats,
        producer_contract_stats,
        producer_contract_runs,
        responsibility_map_drifts,
        responsibility_map_sources,
        responsibility_escalation_deliveries,
        project_runs,
        unassigned_runs,
    })
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: String,
    label: &str,
) -> Result<T> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("{label}: HTTP {}", response.status()));
    }
    Ok(response.json().await?)
}

async fn github_repository_runs(client: &Client, repo: &str) -> Result<Vec<GithubRun>> {
    let url = format!("https://api.github.com/repos/{repo}/actions/runs?per_page=100");
    let data: GithubRunsResponse = fetch_json(client, url, "GitHub Actions 조회 실패").await?;
    Ok(data.workflow_runs)
}

async fn github_repository_responsibility_contracts(
    client: &Client,
    repo: &str,
) -> Result<Option<Vec<RepositoryResponsibilityContract>>> {
    let url = format!("https://api.github.com/repos/{repo}/contents/{RESPONSIBILITY_MAP_PATH}");
    let response = client
        .get(url)
        .header(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github.raw+json"),
        )
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(anyhow!(
            "Responsibility map 조회 실패: HTTP {}",
            response.status()
        ));
    }
    let map: RepositoryResponsibilityMap = serde_json::from_str(&response.text().await?)?;
    let workflows_url =
        format!("https://api.github.com/repos/{repo}/actions/workflows?per_page=100");
    let workflow_data: GithubWorkflowsResponse =
        fetch_json(client, workflows_url, "Workflow inventory 조회 실패").await?;
    let workflow_names: HashMap<String, String> = workflow_data
        .workflows
        .into_iter()
        .map(|workflow| (workflow.path, workflow.name))
        .collect();

    let contracts = map
        .workflows
        .into_iter()
        .map(|(workflow_file, responsibility)| {
            let workflow_path = if workflow_file.starts_with(".github/workflows/") {
                workflow_file.clone()
            } else {
                format!(".github/workflows/{workflow_file}")
            };
            let workflow_name = workflow_names
                .get(&workflow_path)
                .cloned()
                .unwrap_or_else(|| workflow_file.clone());
            let source_binding = responsibility.watchtower_track_binding;
            let (binding_kind, track_key) = normalize_repository_binding(&source_binding);
            RepositoryResponsibilityContract {
                workflow_path,
                workflow_name,
                binding_kind,
                track_key,
                source_binding,
                source_path: RESPONSIBILITY_MAP_PATH.into(),
            }
        })
        .collect();
    Ok(Some(contracts))
}

async fn github_commit_message(
    client: &Client,
    repo: &str,
    sha: &str,
    cache: &mut HashMap<String, Option<String>>,
) -> Option<String> {
    if let Some(value) = cache.get(sha) {
        return value.clone();
    }
    let url = format!("https://api.github.com/repos/{repo}/commits/{sha}");
    let value = fetch_json::<GithubCommitResponse>(client, url, "Commit 조회 실패")
        .await
        .ok()
        .map(|v| v.commit.message);
    cache.insert(sha.to_string(), value.clone());
    value
}

async fn github_prs_for_run(
    client: &Client,
    repo: &str,
    run: &GithubRun,
    cache: &mut HashMap<String, Vec<GithubPull>>,
) -> Vec<GithubPull> {
    if let Some(value) = cache.get(&run.head_sha) {
        return value.clone();
    }
    let mut pulls = Vec::new();
    if !run.pull_requests.is_empty() {
        for pr_ref in run.pull_requests.iter().take(3) {
            let url = format!(
                "https://api.github.com/repos/{repo}/pulls/{}",
                pr_ref.number
            );
            if let Ok(pr) = fetch_json::<GithubPull>(client, url, "PR 조회 실패").await {
                pulls.push(pr);
            }
        }
    } else {
        let url = format!(
            "https://api.github.com/repos/{repo}/commits/{}/pulls",
            run.head_sha
        );
        if let Ok(found) = fetch_json::<Vec<GithubPull>>(client, url, "Commit PR 조회 실패").await
        {
            pulls = found.into_iter().take(3).collect();
        }
    }
    cache.insert(run.head_sha.clone(), pulls.clone());
    pulls
}

fn load_project_aliases(conn: &Connection, project_id: i64) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT ta.alias_key,wt.track_key
         FROM track_aliases ta
         JOIN watch_tracks wt ON wt.id=ta.track_id
         WHERE ta.active=1
           AND wt.active=1
           AND ta.project_id=?
           AND wt.project_id=ta.project_id",
    )?;
    let rows = stmt.query_map(params![project_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn project_rule_matches(
    rules: &[ProjectWorkflowRule],
    project_id: i64,
    repository_id: i64,
    workflow_name: &str,
) -> bool {
    rules.iter().any(|rule| {
        rule.active
            && rule.project_id == project_id
            && rule.workflow_name == workflow_name
            && (rule.repository_id.is_none() || rule.repository_id == Some(repository_id))
    })
}

fn load_fingerprints(
    conn: &Connection,
    project_id: i64,
    repository_id: i64,
) -> Result<Vec<Fingerprint>> {
    let repository_matches_project: bool = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM monitored_repositories
           WHERE id=? AND project_id=?
         )",
        params![repository_id, project_id],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    if !repository_matches_project {
        return Err(anyhow!(
            "Fingerprint 조회 범위의 저장소와 프로젝트가 일치하지 않습니다."
        ));
    }

    let mut stmt = conn.prepare(
        "SELECT wt.track_key,tf.signal_type,tf.pattern,tf.weight
         FROM track_fingerprints tf
         JOIN watch_tracks wt ON wt.id=tf.track_id
         WHERE tf.active=1
           AND wt.active=1
           AND wt.project_id=?
           AND (tf.repository_id IS NULL OR tf.repository_id=?)
           AND (
             tf.repository_id IS NULL
             OR EXISTS(
               SELECT 1 FROM monitored_repositories scope_repo
               WHERE scope_repo.id=tf.repository_id
                 AND scope_repo.project_id=wt.project_id
             )
           )",
    )?;
    let rows = stmt.query_map(params![project_id, repository_id], |row| {
        Ok(Fingerprint {
            track_key: row.get(0)?,
            signal_type: row.get(1)?,
            pattern: row.get(2)?,
            weight: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn fingerprint_evidence(fingerprints: &[Fingerprint], run: &GithubRun) -> Vec<Evidence> {
    let mut out = Vec::new();
    for fingerprint in fingerprints {
        let haystack = match fingerprint.signal_type.as_str() {
            "workflow_name" => run.name.as_str(),
            "workflow_path" => run.path.as_deref().unwrap_or_default(),
            _ => continue,
        };
        if haystack
            .to_lowercase()
            .contains(&fingerprint.pattern.to_lowercase())
        {
            out.push(Evidence {
                track_key: fingerprint.track_key.clone(),
                signal_type: fingerprint.signal_type.clone(),
                score: fingerprint.weight.min(60),
                value: fingerprint.pattern.clone(),
            });
        }
    }
    out
}

fn canonical_evidence_key(key: &str, aliases: &HashMap<String, String>) -> String {
    aliases.get(key).cloned().unwrap_or_else(|| key.to_string())
}

fn resolve_evidence(
    tracks: &[Track],
    aliases: &HashMap<String, String>,
    mut evidence: Vec<Evidence>,
) -> Resolution {
    let alias_evidence: Vec<Evidence> = evidence
        .iter()
        .filter_map(|item| {
            aliases.get(&item.track_key).map(|canonical| Evidence {
                track_key: canonical.clone(),
                signal_type: format!("{}_alias", item.signal_type),
                score: item.score,
                value: format!("{} → {}", item.track_key, canonical),
            })
        })
        .collect();
    evidence.extend(alias_evidence);

    let known: HashMap<&str, i64> = tracks
        .iter()
        .map(|t| (t.track_key.as_str(), t.id))
        .collect();

    for priority_score in [100_i64, 98, 96, 90] {
        let priority_items: Vec<&Evidence> = evidence
            .iter()
            .filter(|item| item.score == priority_score && !item.signal_type.ends_with("_alias"))
            .collect();
        if priority_items.is_empty() {
            continue;
        }

        let priority_keys: HashSet<String> = priority_items
            .iter()
            .map(|item| canonical_evidence_key(&item.track_key, aliases))
            .collect();

        if priority_keys.len() > 1 {
            let mut keys = priority_keys.into_iter().collect::<Vec<_>>();
            keys.sort();
            return Resolution {
                status: "conflict".into(),
                track_id: None,
                confidence: None,
                source: Some("explicit_conflict".into()),
                reason: Some(format!(
                    "동일 우선순위 Track Key가 충돌합니다: {}",
                    keys.join(", ")
                )),
                evidence,
            };
        }

        let key = priority_keys.into_iter().next().expect("priority key");
        if let Some(track_id) = known.get(key.as_str()).copied() {
            let best = priority_items
                .iter()
                .find(|item| canonical_evidence_key(&item.track_key, aliases) == key)
                .expect("priority evidence");
            return Resolution {
                status: "assigned".into(),
                track_id: Some(track_id),
                confidence: Some(priority_score),
                source: Some(best.signal_type.clone()),
                reason: Some(format!("{} → {}", best.signal_type, key)),
                evidence,
            };
        }

        if priority_score >= 96 {
            return Resolution {
                status: "unassigned".into(),
                track_id: None,
                confidence: Some(priority_score),
                source: priority_items.first().map(|item| item.signal_type.clone()),
                reason: Some(format!("등록되지 않은 Track Key 발견: {}", key)),
                evidence,
            };
        }
    }

    let mut scores: HashMap<String, i64> = HashMap::new();
    for item in evidence
        .iter()
        .filter(|item| !item.signal_type.ends_with("_alias"))
    {
        let key = canonical_evidence_key(&item.track_key, aliases);
        if known.contains_key(key.as_str()) {
            *scores.entry(key).or_default() += item.score;
        }
    }
    let mut ranked: Vec<(String, i64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    if let Some((key, score)) = ranked.first() {
        let second = ranked.get(1).map(|v| v.1).unwrap_or(0);
        if *score >= 70 && *score - second >= 30 {
            return Resolution {
                status: "assigned".into(),
                track_id: known.get(key.as_str()).copied(),
                confidence: Some((*score).min(100)),
                source: Some("inference".into()),
                reason: Some(format!("복합 신호 {}점 (2위 {}점)", score, second)),
                evidence,
            };
        }
    }

    Resolution {
        status: "unassigned".into(),
        track_id: None,
        confidence: ranked.first().map(|v| v.1.min(100)),
        source: None,
        reason: Some("확정 가능한 Track Key 근거가 없습니다.".into()),
        evidence,
    }
}

async fn resolve_run(
    client: &Client,
    repo: &str,
    run: &GithubRun,
    project_id: i64,
    repository_id: i64,
    tracks: &[Track],
    project_rules: &[ProjectWorkflowRule],
    aliases: &HashMap<String, String>,
    fingerprints: &[Fingerprint],
    commit_cache: &mut HashMap<String, Option<String>>,
    pr_cache: &mut HashMap<String, Vec<GithubPull>>,
) -> Result<Resolution> {
    if project_rule_matches(project_rules, project_id, repository_id, &run.name) {
        return Ok(Resolution {
            status: "project".into(),
            track_id: None,
            confidence: Some(100),
            source: Some("project_workflow".into()),
            reason: Some(format!("프로젝트 공용 CI: {}", run.name)),
            evidence: Vec::new(),
        });
    }

    let mut evidence = Vec::new();

    if let Some(key) = run.display_title.as_deref().and_then(extract_marker) {
        evidence.push(Evidence {
            track_key: key,
            signal_type: "run_name".into(),
            score: 100,
            value: run.display_title.clone().unwrap_or_default(),
        });
    }

    if let Some(branch) = run.head_branch.as_deref() {
        for track in tracks {
            if branch_has_key(branch, &track.track_key) {
                evidence.push(Evidence {
                    track_key: track.track_key.clone(),
                    signal_type: "branch".into(),
                    score: 90,
                    value: branch.to_string(),
                });
            }
        }
    }

    let pulls = github_prs_for_run(client, repo, run, pr_cache).await;
    for pr in pulls {
        if let Some(key) = pr.body.as_deref().and_then(extract_track_trailer) {
            evidence.push(Evidence {
                track_key: key,
                signal_type: "pr_marker".into(),
                score: 98,
                value: format!("PR #{}", pr.number),
            });
        }
    }

    if let Some(message) = github_commit_message(client, repo, &run.head_sha, commit_cache).await {
        if let Some(key) = extract_track_trailer(&message) {
            evidence.push(Evidence {
                track_key: key,
                signal_type: "commit_marker".into(),
                score: 96,
                value: run.head_sha.clone(),
            });
        }
    }

    evidence.extend(fingerprint_evidence(fingerprints, run));

    return Ok(resolve_evidence(tracks, aliases, evidence));
}

fn persist_resolution_with_trigger(
    conn: &Connection,
    run_id: i64,
    resolution: &Resolution,
    now: &str,
    trigger: Option<&str>,
) -> Result<()> {
    let previous: Option<(
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
    )> = if trigger.is_some() {
        conn.query_row(
                "SELECT wr.repository_id,wr.resolution_status,wt.track_key,ra.source,ra.confidence,ra.manual
                 FROM workflow_runs wr
                 LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
                 LEFT JOIN watch_tracks wt ON wt.id=ra.track_id
                 WHERE wr.run_id=?",
                params![run_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?
    } else {
        let manual: Option<i64> = conn
            .query_row(
                "SELECT manual FROM run_assignments WHERE run_id=?",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?;
        Some((0, String::new(), None, None, None, manual))
    };
    let Some((
        repository_id,
        previous_status,
        previous_track_key,
        previous_source,
        previous_confidence,
        manual,
    )) = previous
    else {
        return Err(anyhow!("Run을 찾지 못했습니다."));
    };
    if manual == Some(1) {
        return Ok(());
    }

    let previous_evidence: Vec<Evidence> = {
        let mut stmt = conn.prepare(
            "SELECT track_key,signal_type,score,value
             FROM run_evidence
             WHERE run_id=?
             ORDER BY score DESC,id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok(Evidence {
                track_key: row.get(0)?,
                signal_type: row.get(1)?,
                score: row.get(2)?,
                value: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    conn.execute("DELETE FROM run_evidence WHERE run_id=?", params![run_id])?;
    for item in &resolution.evidence {
        conn.execute(
            "INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) VALUES(?,?,?,?,?,?)",
            params![run_id, item.track_key, item.signal_type, item.score, item.value, now],
        )?;
    }
    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status=?,last_resolution_attempt_at=?
         WHERE run_id=?",
        params![resolution.status, now, run_id],
    )?;
    if let Some(track_id) = resolution.track_id {
        conn.execute(
            "INSERT INTO run_assignments(run_id,track_id,confidence,source,reason,manual,assigned_at)
             VALUES(?,?,?,?,?,0,?)
             ON CONFLICT(run_id) DO UPDATE SET track_id=excluded.track_id,confidence=excluded.confidence,source=excluded.source,reason=excluded.reason,manual=0,assigned_at=excluded.assigned_at",
            params![
                run_id,
                track_id,
                resolution.confidence.unwrap_or(0),
                resolution.source.as_deref().unwrap_or("resolver"),
                resolution.reason.as_deref().unwrap_or(""),
                now
            ],
        )?;
    } else {
        conn.execute(
            "DELETE FROM run_assignments WHERE run_id=? AND manual=0",
            params![run_id],
        )?;
    }

    if let Some(trigger) = trigger {
        let next_track_key: Option<String> = if let Some(track_id) = resolution.track_id {
            conn.query_row(
                "SELECT track_key FROM watch_tracks WHERE id=?",
                params![track_id],
                |row| row.get(0),
            )
            .optional()?
        } else {
            None
        };
        let decision_changed =
            previous_status != resolution.status || previous_track_key != next_track_key;
        if decision_changed {
            conn.execute(
                "INSERT INTO resolution_reconciliation_audit(
                   run_id,repository_id,trigger,
                   from_status,from_track_key,from_source,from_confidence,
                   to_status,to_track_key,to_source,to_confidence,to_reason,
                   previous_evidence_json,evidence_json,reconciled_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    run_id,
                    repository_id,
                    trigger,
                    previous_status,
                    previous_track_key,
                    previous_source,
                    previous_confidence,
                    resolution.status,
                    next_track_key,
                    resolution.source,
                    resolution.confidence,
                    resolution.reason,
                    serde_json::to_string(&previous_evidence)?,
                    serde_json::to_string(&resolution.evidence)?,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

fn persist_resolution(
    conn: &Connection,
    run_id: i64,
    resolution: &Resolution,
    now: &str,
) -> Result<()> {
    persist_resolution_with_trigger(conn, run_id, resolution, now, None)
}

fn upsert_run(conn: &Connection, repository_id: i64, run: &GithubRun, now: &str) -> Result<()> {
    let title = run
        .display_title
        .clone()
        .unwrap_or_else(|| run.name.clone());
    conn.execute(
        "INSERT INTO workflow_runs(run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,created_at,run_started_at,updated_at,last_seen_at)
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
         ON CONFLICT(run_id) DO UPDATE SET
           repository_id=excluded.repository_id,workflow_id=excluded.workflow_id,workflow_name=excluded.workflow_name,
           workflow_path=excluded.workflow_path,display_title=excluded.display_title,event=excluded.event,
           head_branch=excluded.head_branch,head_sha=excluded.head_sha,run_number=excluded.run_number,
           run_attempt=excluded.run_attempt,status=excluded.status,conclusion=excluded.conclusion,
           html_url=excluded.html_url,run_started_at=excluded.run_started_at,updated_at=excluded.updated_at,last_seen_at=excluded.last_seen_at",
        params![
            run.id,
            repository_id,
            run.workflow_id,
            run.name,
            run.path,
            title,
            run.event,
            run.head_branch,
            run.head_sha,
            run.run_number,
            run.run_attempt,
            run.status,
            run.conclusion,
            run.html_url,
            run.created_at,
            run.run_started_at,
            run.updated_at,
            now
        ],
    )?;
    conn.execute(
        "INSERT INTO run_attempts(run_id,run_attempt,status,conclusion,started_at,updated_at)
         VALUES(?,?,?,?,?,?)
         ON CONFLICT(run_id,run_attempt) DO UPDATE SET status=excluded.status,conclusion=excluded.conclusion,started_at=excluded.started_at,updated_at=excluded.updated_at",
        params![
            run.id,
            run.run_attempt,
            run.status,
            run.conclusion,
            run.run_started_at,
            run.updated_at
        ],
    )?;
    Ok(())
}

fn load_stored_unresolved_runs(
    conn: &Connection,
    repository_id: i64,
    limit: i64,
) -> Result<Vec<GithubRun>> {
    let mut stmt = conn.prepare(
        "SELECT run_id,workflow_id,workflow_name,workflow_path,display_title,event,head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,created_at,run_started_at,updated_at
         FROM workflow_runs
         WHERE repository_id=?
           AND ignored=0
           AND COALESCE(workflow_path,'') NOT LIKE 'dynamic/dependabot/%'
           AND resolution_status IN ('unassigned','conflict')
           AND NOT EXISTS(
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )
         ORDER BY
           CASE WHEN last_resolution_attempt_at IS NULL THEN 0 ELSE 1 END,
           last_resolution_attempt_at ASC,
           created_at DESC
         LIMIT ?",
    )?;
    let rows = stmt.query_map(params![repository_id, limit], |row| {
        Ok(GithubRun {
            id: row.get(0)?,
            workflow_id: row.get(1)?,
            name: row.get(2)?,
            path: row.get(3)?,
            display_title: row.get(4)?,
            event: row.get(5)?,
            head_branch: row.get(6)?,
            head_sha: row.get(7)?,
            run_number: row.get(8)?,
            run_attempt: row.get(9)?,
            status: row.get(10)?,
            conclusion: row.get(11)?,
            html_url: row.get(12)?,
            created_at: row.get(13)?,
            run_started_at: row.get(14)?,
            updated_at: row.get(15)?,
            pull_requests: Vec::new(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn mark_notified(
    conn: &Connection,
    track_id: i64,
    run_id: i64,
    attempt: i64,
    event_type: &str,
    now: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO notifications_v2(track_id,run_id,run_attempt,event_type,notified_at) VALUES(?,?,?,?,?)",
        params![track_id, run_id, attempt, event_type, now],
    )? > 0)
}

fn send_notification(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn responsibility_escalation_event_type(level: &str) -> Option<&'static str> {
    match level {
        "warning" => Some("warning"),
        "critical" => Some("critical"),
        _ => None,
    }
}

fn responsibility_source_synced(conn: &Connection, repository_id: i64) -> Result<bool> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM repository_responsibility_sources WHERE repository_id=?",
            params![repository_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(status.as_deref() == Some("synced"))
}

fn reserve_responsibility_escalation_delivery(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
    event_type: &str,
    now: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "INSERT INTO responsibility_escalation_deliveries(
           review_key,fingerprint,project_id,repository_id,workflow_name,event_type,
           escalation_level,sla_status,reason,status,attempts,last_error,
           first_attempt_at,last_attempt_at,emitted_at
         ) VALUES(?,?,?,?,?,?,?,?,?,'pending',1,NULL,?,?,NULL)
         ON CONFLICT(review_key,fingerprint,event_type) DO UPDATE SET
           project_id=excluded.project_id,
           repository_id=excluded.repository_id,
           workflow_name=excluded.workflow_name,
           escalation_level=excluded.escalation_level,
           sla_status=excluded.sla_status,
           reason=excluded.reason,
           status='pending',
           attempts=responsibility_escalation_deliveries.attempts+1,
           last_error=NULL,
           last_attempt_at=excluded.last_attempt_at
         WHERE responsibility_escalation_deliveries.status!='emitted'
           AND responsibility_escalation_deliveries.attempts < ?",
        params![
            drift.review_key,
            drift.fingerprint,
            drift.project_id,
            drift.repository_id,
            drift.workflow_name,
            event_type,
            drift.escalation_level,
            drift.sla_status,
            drift.escalation_reason,
            now,
            now,
            RESPONSIBILITY_ESCALATION_MAX_ATTEMPTS,
        ],
    )?;
    Ok(changed > 0)
}

fn finish_responsibility_escalation_delivery(
    conn: &Connection,
    review_key: &str,
    fingerprint: &str,
    event_type: &str,
    now: &str,
    result: std::result::Result<(), String>,
) -> Result<()> {
    match result {
        Ok(()) => {
            conn.execute(
                "UPDATE responsibility_escalation_deliveries
                 SET status='emitted',last_error=NULL,emitted_at=?,last_attempt_at=?
                 WHERE review_key=? AND fingerprint=? AND event_type=?",
                params![now, now, review_key, fingerprint, event_type],
            )?;
        }
        Err(error) => {
            conn.execute(
                "UPDATE responsibility_escalation_deliveries
                 SET status='failed',last_error=?,last_attempt_at=?
                 WHERE review_key=? AND fingerprint=? AND event_type=?",
                params![error, now, review_key, fingerprint, event_type],
            )?;
        }
    }
    Ok(())
}

fn responsibility_escalation_deliveries(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<ResponsibilityEscalationDelivery>> {
    let mut stmt = conn.prepare(
        "SELECT d.id,d.project_id,d.repository_id,mr.repo,d.workflow_name,
                d.review_key,d.fingerprint,d.event_type,d.escalation_level,d.sla_status,
                d.reason,d.status,d.attempts,d.last_error,d.first_attempt_at,
                d.last_attempt_at,d.emitted_at
         FROM responsibility_escalation_deliveries d
         JOIN monitored_repositories mr ON mr.id=d.repository_id
         ORDER BY COALESCE(d.emitted_at,d.last_attempt_at) DESC,d.id DESC
         LIMIT ?",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(ResponsibilityEscalationDelivery {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repository_id: row.get(2)?,
            repository: row.get(3)?,
            workflow_name: row.get(4)?,
            review_key: row.get(5)?,
            fingerprint: row.get(6)?,
            event_type: row.get(7)?,
            escalation_level: row.get(8)?,
            sla_status: row.get(9)?,
            reason: row.get(10)?,
            status: row.get(11)?,
            attempts: row.get(12)?,
            last_error: row.get(13)?,
            first_attempt_at: row.get(14)?,
            last_attempt_at: row.get(15)?,
            emitted_at: row.get(16)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn notify_responsibility_escalations(
    app: &AppHandle,
    conn: &Connection,
    now: DateTime<Utc>,
) -> Result<()> {
    let now_text = now.to_rfc3339();
    for drift in responsibility_map_drifts(conn)? {
        let Some(event_type) = responsibility_escalation_event_type(&drift.escalation_level) else {
            continue;
        };
        let policy = responsibility_review_policy(conn, drift.project_id)?;
        if (event_type == "warning" && !policy.notify_warning)
            || (event_type == "critical" && !policy.notify_critical)
        {
            continue;
        }
        if !responsibility_source_synced(conn, drift.repository_id)? {
            continue;
        }
        if !reserve_responsibility_escalation_delivery(conn, &drift, event_type, &now_text)? {
            continue;
        }
        let title = format!(
            "[Responsibility] {} — {}",
            drift.escalation_level.to_uppercase(),
            drift.workflow_name
        );
        let body = format!(
            "{} · {} · {}",
            drift.repository,
            drift.review_priority.to_uppercase(),
            drift
                .escalation_reason
                .as_deref()
                .unwrap_or("Responsibility review escalation")
        );
        let result = app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|error| error.to_string());
        finish_responsibility_escalation_delivery(
            conn,
            &drift.review_key,
            &drift.fingerprint,
            event_type,
            &now_text,
            result,
        )?;
    }
    Ok(())
}

fn notify_runs(
    app: &AppHandle,
    conn: &Connection,
    tracks: &[Track],
    now: DateTime<Utc>,
) -> Result<()> {
    let now_str = now.to_rfc3339();
    for track in tracks {
        let runs = runs_for_track(conn, track.id, 100)?;
        for run in runs {
            if run.status == "completed" {
                let event_type = match run.conclusion.as_deref() {
                    Some("success") => "green",
                    Some(
                        "failure" | "cancelled" | "timed_out" | "action_required"
                        | "startup_failure" | "stale",
                    ) => "red",
                    _ => "done",
                };
                if mark_notified(
                    conn,
                    track.id,
                    run.id,
                    run.run_attempt,
                    event_type,
                    &now_str,
                )? {
                    let label = match event_type {
                        "green" => "GREEN",
                        "red" => "RED",
                        _ => "DONE",
                    };
                    send_notification(
                        app,
                        &format!("[{}] CI 완료 — {}", track.name, label),
                        &format!("{} · {}", run.repository, run.workflow_name),
                    );
                }
            } else if run.status == "in_progress"
                && run.elapsed_seconds >= track.long_ci_minutes.saturating_mul(60)
                && mark_notified(conn, track.id, run.id, run.run_attempt, "long", &now_str)?
            {
                send_notification(
                    app,
                    &format!("[{}] 장기 CI 감지", track.name),
                    &format!(
                        "{} · {} · {}분 초과",
                        run.repository, run.workflow_name, track.long_ci_minutes
                    ),
                );
            }
        }
    }
    Ok(())
}

async fn poll_all(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    if state.poll_in_flight.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = poll_all_inner(app, &state).await;
    state.poll_in_flight.store(false, Ordering::SeqCst);
    result
}

async fn poll_all_inner(app: &AppHandle, state: &AppState) -> Result<()> {
    let token = github_token()?;
    let client = github_client(&token)?;
    let repositories = {
        let conn = db(state)?;
        list_repositories(&conn, true)?
    };
    if repositories.is_empty() {
        return Ok(());
    }
    let tracks = {
        let conn = db(state)?;
        list_tracks(&conn, true)?
    };
    let project_rules = {
        let conn = db(state)?;
        list_project_workflow_rules(&conn)?
    };
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    for repository in repositories {
        match github_repository_runs(&client, &repository.repo).await {
            Err(err) => {
                let conn = db(state)?;
                conn.execute(
                    "UPDATE monitored_repositories SET running_count=0,queued_count=0,last_polled_at=?,last_error=?,updated_at=? WHERE id=?",
                    params![now_str, err.to_string(), now_str, repository.id],
                )?;
            }
            Ok(runs) => {
                let running_count =
                    runs.iter().filter(|r| r.status == "in_progress").count() as i64;
                let queued_count = runs
                    .iter()
                    .filter(|r| {
                        matches!(
                            r.status.as_str(),
                            "queued" | "requested" | "pending" | "waiting"
                        )
                    })
                    .count() as i64;
                {
                    let conn = db(state)?;
                    conn.execute(
                        "UPDATE monitored_repositories SET running_count=?,queued_count=?,last_polled_at=?,last_successful_poll_at=?,last_error=NULL,updated_at=? WHERE id=?",
                        params![running_count, queued_count, now_str, now_str, now_str, repository.id],
                    )?;
                    for run in &runs {
                        upsert_run(&conn, repository.id, run, &now_str)?;
                    }
                }

                match github_repository_responsibility_contracts(&client, &repository.repo).await {
                    Ok(Some(contracts)) => {
                        let conn = db(state)?;
                        replace_repository_responsibility_contracts(
                            &conn,
                            repository.id,
                            &contracts,
                            &now_str,
                        )?;
                        update_repository_responsibility_source(
                            &conn,
                            repository.id,
                            "synced",
                            &now_str,
                            None,
                        )?;
                    }
                    Ok(None) => {
                        let conn = db(state)?;
                        update_repository_responsibility_source(
                            &conn,
                            repository.id,
                            "not_found",
                            &now_str,
                            None,
                        )?;
                    }
                    Err(error) => {
                        let conn = db(state)?;
                        let error_text = error.to_string();
                        update_repository_responsibility_source(
                            &conn,
                            repository.id,
                            "error",
                            &now_str,
                            Some(&error_text),
                        )?;
                    }
                }

                let fingerprints = {
                    let conn = db(state)?;
                    load_fingerprints(&conn, repository.project_id, repository.id)?
                };
                let aliases = {
                    let conn = db(state)?;
                    load_project_aliases(&conn, repository.project_id)?
                };
                let repository_tracks: Vec<Track> = tracks
                    .iter()
                    .filter(|track| track.project_id == repository.project_id)
                    .cloned()
                    .collect();
                let mut commit_cache = HashMap::new();
                let mut pr_cache = HashMap::new();

                for run in &runs {
                    let should_resolve = {
                        let conn = db(state)?;
                        let manual: Option<i64> = conn
                            .query_row(
                                "SELECT manual FROM run_assignments WHERE run_id=?",
                                params![run.id],
                                |row| row.get(0),
                            )
                            .optional()?;
                        manual != Some(1)
                    };
                    if !should_resolve {
                        continue;
                    }
                    let resolution = resolve_run(
                        &client,
                        &repository.repo,
                        run,
                        repository.project_id,
                        repository.id,
                        &repository_tracks,
                        &project_rules,
                        &aliases,
                        &fingerprints,
                        &mut commit_cache,
                        &mut pr_cache,
                    )
                    .await?;
                    let conn = db(state)?;
                    persist_resolution(&conn, run.id, &resolution, &now_str)?;
                }

                // Old completed runs can fall out of GitHub's recent-100 window while still
                // remaining unresolved locally. Re-evaluate a bounded batch each poll so
                // PR/commit markers and newly-added project rules eventually backfill them.
                let recent_ids: HashSet<i64> = runs.iter().map(|run| run.id).collect();
                let stored_unresolved = {
                    let conn = db(state)?;
                    load_stored_unresolved_runs(&conn, repository.id, HISTORICAL_RECONCILE_BATCH)?
                };
                for run in stored_unresolved {
                    if recent_ids.contains(&run.id) {
                        continue;
                    }
                    let resolution = resolve_run(
                        &client,
                        &repository.repo,
                        &run,
                        repository.project_id,
                        repository.id,
                        &repository_tracks,
                        &project_rules,
                        &aliases,
                        &fingerprints,
                        &mut commit_cache,
                        &mut pr_cache,
                    )
                    .await?;
                    let conn = db(state)?;
                    persist_resolution_with_trigger(
                        &conn,
                        run.id,
                        &resolution,
                        &now_str,
                        Some("historical_reconcile"),
                    )?;
                }
            }
        }
    }

    let conn = db(state)?;
    notify_runs(app, &conn, &tracks, now)?;
    notify_responsibility_escalations(app, &conn, now)?;
    let dashboard = build_dashboard(state)?;
    let was_congested: bool = conn.query_row(
        "SELECT queue_congested FROM app_settings WHERE id=1",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    let now_congested =
        dashboard.queued_count >= dashboard.settings.queue_congestion_threshold as usize;
    if now_congested && !was_congested {
        send_notification(
            app,
            "GitHub Actions Queue 혼잡",
            &format!(
                "Queued {} · Running {} · 설정 기준 {}",
                dashboard.queued_count,
                dashboard.running_count,
                dashboard.settings.queue_congestion_threshold
            ),
        );
    }
    if now_congested != was_congested {
        conn.execute(
            "UPDATE app_settings SET queue_congested=? WHERE id=1",
            params![if now_congested { 1 } else { 0 }],
        )?;
    }
    Ok(())
}

#[tauri::command]
fn get_dashboard(state: State<'_, AppState>) -> std::result::Result<Dashboard, String> {
    build_dashboard(&state).map_err(|e| e.to_string())
}

fn current_responsibility_drift(
    conn: &Connection,
    review_key: &str,
) -> Result<ResponsibilityMapDrift> {
    responsibility_map_drifts(conn)?
        .into_iter()
        .find(|item| item.review_key == review_key)
        .ok_or_else(|| anyhow!("검토 대상 Responsibility Drift가 더 이상 존재하지 않습니다."))
}

fn exact_project_rule(
    conn: &Connection,
    project_id: i64,
    repository_id: i64,
    workflow_name: &str,
) -> Result<Option<ProjectWorkflowRule>> {
    Ok(conn
        .query_row(
            "SELECT id,project_id,repository_id,workflow_name,active
             FROM project_workflow_rules
             WHERE project_id=? AND repository_id=? AND workflow_name=? AND active=1",
            params![project_id, repository_id, workflow_name],
            |row| {
                Ok(ProjectWorkflowRule {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    repository_id: row.get(2)?,
                    workflow_name: row.get(3)?,
                    active: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()?)
}

fn exact_dynamic_rule(
    conn: &Connection,
    project_id: i64,
    repository_id: i64,
    workflow_name: &str,
) -> Result<Option<DynamicWorkflowRule>> {
    Ok(conn
        .query_row(
            "SELECT id,project_id,repository_id,workflow_name,active,protected
             FROM dynamic_workflow_rules
             WHERE project_id=? AND repository_id=? AND workflow_name=? AND active=1",
            params![project_id, repository_id, workflow_name],
            |row| {
                Ok(DynamicWorkflowRule {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    repository_id: row.get(2)?,
                    workflow_name: row.get(3)?,
                    active: row.get::<_, i64>(4)? != 0,
                    protected: row.get::<_, i64>(5)? != 0,
                })
            },
        )
        .optional()?)
}

fn matching_global_project_rule(
    conn: &Connection,
    project_id: i64,
    workflow_name: &str,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM project_workflow_rules
           WHERE project_id=? AND repository_id IS NULL AND workflow_name=? AND active=1
         )",
        params![project_id, workflow_name],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )
    .map_err(Into::into)
}

fn matching_global_dynamic_rule(
    conn: &Connection,
    project_id: i64,
    workflow_name: &str,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM dynamic_workflow_rules
           WHERE project_id=? AND repository_id IS NULL AND workflow_name=? AND active=1
         )",
        params![project_id, workflow_name],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )
    .map_err(Into::into)
}

fn resolution_preview_for_drift(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<ResponsibilityResolutionPreview> {
    let mut executable = true;
    let mut blocked_reason = None;
    let (action, changes) = match drift.recommended_action.as_str() {
        "review_add_project_wide_rule" => (
            "add_project_wide_rule",
            vec![format!(
                "Add repository-scoped Project-wide rule: {}",
                drift.workflow_name
            )],
        ),
        "review_add_dynamic_rule" => (
            "add_dynamic_rule",
            vec![format!(
                "Add repository-scoped protected Dynamic rule: {}",
                drift.workflow_name
            )],
        ),
        "review_reclassify_project_wide" => {
            if matching_global_dynamic_rule(conn, drift.project_id, &drift.workflow_name)? {
                executable = false;
                blocked_reason = Some(
                    "현재 Dynamic 선언이 Project 전체 범위입니다. 단일 Repository drift 해결을 위해 전역 규칙을 자동 변경하지 않습니다.".into(),
                );
            } else if exact_dynamic_rule(
                conn,
                drift.project_id,
                drift.repository_id,
                &drift.workflow_name,
            )?
            .is_none()
            {
                executable = false;
                blocked_reason =
                    Some("변경할 repository-scoped Dynamic 규칙을 찾지 못했습니다.".into());
            }
            (
                "reclassify_to_project_wide",
                vec![
                    format!(
                        "Remove repository-scoped Dynamic rule: {}",
                        drift.workflow_name
                    ),
                    format!(
                        "Add repository-scoped Project-wide rule: {}",
                        drift.workflow_name
                    ),
                ],
            )
        }
        "review_reclassify_dynamic" => {
            if matching_global_project_rule(conn, drift.project_id, &drift.workflow_name)? {
                executable = false;
                blocked_reason = Some(
                    "현재 Project-wide 선언이 Project 전체 범위입니다. 단일 Repository drift 해결을 위해 전역 규칙을 자동 변경하지 않습니다.".into(),
                );
            } else if exact_project_rule(
                conn,
                drift.project_id,
                drift.repository_id,
                &drift.workflow_name,
            )?
            .is_none()
            {
                executable = false;
                blocked_reason =
                    Some("변경할 repository-scoped Project-wide 규칙을 찾지 못했습니다.".into());
            }
            (
                "reclassify_to_dynamic",
                vec![
                    format!(
                        "Remove repository-scoped Project-wide rule: {}",
                        drift.workflow_name
                    ),
                    format!(
                        "Add repository-scoped protected Dynamic rule: {}",
                        drift.workflow_name
                    ),
                ],
            )
        }
        "review_remove_or_confirm_stale_rule" => {
            let exact_exists = match drift.watchtower_binding.as_deref() {
                Some("project-wide") => {
                    if matching_global_project_rule(conn, drift.project_id, &drift.workflow_name)? {
                        executable = false;
                        blocked_reason = Some(
                            "Stale 선언이 Project 전체 범위입니다. 다른 Repository에 영향을 줄 수 있어 자동 제거하지 않습니다.".into(),
                        );
                        false
                    } else {
                        exact_project_rule(
                            conn,
                            drift.project_id,
                            drift.repository_id,
                            &drift.workflow_name,
                        )?
                        .is_some()
                    }
                }
                Some("dynamic") => {
                    if matching_global_dynamic_rule(conn, drift.project_id, &drift.workflow_name)? {
                        executable = false;
                        blocked_reason = Some(
                            "Stale Dynamic 선언이 Project 전체 범위입니다. 다른 Repository에 영향을 줄 수 있어 자동 제거하지 않습니다.".into(),
                        );
                        false
                    } else {
                        exact_dynamic_rule(
                            conn,
                            drift.project_id,
                            drift.repository_id,
                            &drift.workflow_name,
                        )?
                        .is_some()
                    }
                }
                _ => false,
            };
            if executable && !exact_exists {
                executable = false;
                blocked_reason = Some("제거할 repository-scoped 규칙을 찾지 못했습니다.".into());
            }
            (
                "remove_stale_rule",
                vec![format!(
                    "Remove repository-scoped stale {} rule: {}",
                    drift
                        .watchtower_binding
                        .as_deref()
                        .unwrap_or("responsibility"),
                    drift.workflow_name
                )],
            )
        }
        _ => {
            executable = false;
            blocked_reason = Some(match drift.recommended_action.as_str() {
                "review_track_registry_or_map" => {
                    "Canonical Track registry 또는 repository responsibility map을 먼저 검토해야 합니다."
                }
                "review_producer_run_name" => {
                    "Producer run-name의 [WT:*] evidence를 repository에서 수정해야 합니다."
                }
                "review_remove_conflicting_responsibility_rule" => {
                    "Static Track 계약과 충돌하는 책임 규칙은 자동 제거하지 않습니다. Track evidence를 먼저 확인해야 합니다."
                }
                _ => "이 drift 유형은 WatchTower 내부 자동 변경 대상이 아닙니다.",
            }
            .into());
            ("blocked", Vec::new())
        }
    };

    Ok(ResponsibilityResolutionPreview {
        review_key: drift.review_key.clone(),
        fingerprint: drift.fingerprint.clone(),
        workflow_name: drift.workflow_name.clone(),
        repository: drift.repository.clone(),
        repository_contract: drift.repository_binding.clone(),
        watchtower_contract: drift.watchtower_binding.clone(),
        action: action.into(),
        changes,
        invariants: vec![
            "Canonical Tracks are not created or modified.".into(),
            "workflow_runs are not deleted.".into(),
            "Manual run assignments are preserved.".into(),
            "Producer YAML and repository responsibility map are not modified.".into(),
        ],
        executable,
        blocked_reason,
    })
}

fn current_watchtower_responsibility_binding(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<Option<String>> {
    let project_rules = list_project_workflow_rules(conn)?;
    let dynamic_rules = list_dynamic_workflow_rules(conn)?;
    let project_declared = project_rule_matches(
        &project_rules,
        drift.project_id,
        drift.repository_id,
        &drift.workflow_name,
    );
    let dynamic_declared = dynamic_rule_matches(
        &dynamic_rules,
        drift.project_id,
        drift.repository_id,
        &drift.workflow_name,
    );
    if project_declared && dynamic_declared {
        return Ok(Some("project-wide + dynamic".into()));
    }
    if project_declared {
        return Ok(Some("project-wide".into()));
    }
    if dynamic_declared {
        return Ok(Some("dynamic".into()));
    }
    Ok(latest_workflow_run_title(
        conn,
        drift.repository_id,
        &drift.workflow_path,
        &drift.workflow_name,
    )?
    .as_deref()
    .and_then(extract_marker)
    .map(|key| format!("static:{key}")))
}

fn insert_resolution_audit(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
    action: &str,
    requested_fingerprint: &str,
    current_fingerprint: &str,
    resulting_watchtower_binding: Option<&str>,
    result: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO responsibility_resolution_audit(
           review_key,fingerprint,project_id,repository_id,workflow_name,drift_type,
           action,before_repository_binding,before_watchtower_binding,
           after_repository_binding,after_watchtower_binding,
           requested_fingerprint,current_fingerprint,expected_repository_binding,
           resulting_watchtower_binding,result,actor,created_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            drift.review_key,
            current_fingerprint,
            drift.project_id,
            drift.repository_id,
            drift.workflow_name,
            drift.drift_type,
            action,
            drift.repository_binding,
            drift.watchtower_binding,
            drift.repository_binding,
            resulting_watchtower_binding,
            requested_fingerprint,
            current_fingerprint,
            drift.repository_binding,
            resulting_watchtower_binding,
            result,
            "local-user",
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn responsibility_resolution_history(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<ResponsibilityResolutionAuditEntry>> {
    let mut stmt = conn.prepare(
        "SELECT rra.id,rra.project_id,p.name,rra.repository_id,mr.repo,
                rra.workflow_name,rra.review_key,rra.drift_type,rra.action,rra.result,
                rra.actor,rra.created_at,
                COALESCE(rra.requested_fingerprint,rra.fingerprint),
                COALESCE(rra.current_fingerprint,rra.fingerprint),
                COALESCE(rra.expected_repository_binding,rra.before_repository_binding),
                rra.before_watchtower_binding,
                COALESCE(rra.resulting_watchtower_binding,rra.after_watchtower_binding)
         FROM responsibility_resolution_audit rra
         JOIN projects p ON p.id=rra.project_id
         JOIN monitored_repositories mr ON mr.id=rra.repository_id
         ORDER BY rra.id DESC
         LIMIT ?",
    )?;
    let rows = stmt.query_map(params![limit.clamp(1, 500)], |row| {
        let requested_fingerprint: String = row.get(12)?;
        let current_fingerprint: String = row.get(13)?;
        Ok(ResponsibilityResolutionAuditEntry {
            id: row.get(0)?,
            project_id: row.get(1)?,
            project_name: row.get(2)?,
            repository_id: row.get(3)?,
            repository: row.get(4)?,
            workflow_name: row.get(5)?,
            review_key: row.get(6)?,
            drift_type: row.get(7)?,
            action: row.get(8)?,
            result: row.get(9)?,
            actor: row.get(10)?,
            created_at: row.get(11)?,
            stale: requested_fingerprint != current_fingerprint,
            requested_fingerprint,
            current_fingerprint,
            repository_contract: row.get(14)?,
            before_watchtower_contract: row.get(15)?,
            after_watchtower_contract: row.get(16)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn add_project_rule_for_resolution(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
         VALUES(?,?,?,1,?)",
        params![drift.project_id, drift.repository_id, drift.workflow_name, now],
    )?;
    conn.execute(
        "UPDATE project_workflow_rules SET active=1
         WHERE project_id=? AND repository_id=? AND workflow_name=?",
        params![drift.project_id, drift.repository_id, drift.workflow_name],
    )?;
    conn.execute(
        "DELETE FROM run_assignments
         WHERE manual=0 AND run_id IN (
           SELECT wr.run_id FROM workflow_runs wr
           WHERE wr.repository_id=? AND wr.workflow_name=?
         )",
        params![drift.repository_id, drift.workflow_name],
    )?;
    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='project'
         WHERE ignored=0 AND repository_id=? AND workflow_name=?
           AND NOT EXISTS(
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        params![drift.repository_id, drift.workflow_name],
    )?;
    Ok(())
}

fn remove_project_rule_for_resolution(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<()> {
    let deleted = conn.execute(
        "DELETE FROM project_workflow_rules
         WHERE project_id=? AND repository_id=? AND workflow_name=?",
        params![drift.project_id, drift.repository_id, drift.workflow_name],
    )?;
    if deleted == 0 {
        return Err(anyhow!(
            "제거할 repository-scoped Project-wide 규칙을 찾지 못했습니다."
        ));
    }
    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE ignored=0 AND resolution_status='project'
           AND repository_id=? AND workflow_name=?
           AND NOT EXISTS(
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )
           AND NOT EXISTS(
             SELECT 1 FROM project_workflow_rules pwr
             WHERE pwr.active=1
               AND pwr.project_id=?
               AND pwr.workflow_name=workflow_runs.workflow_name
               AND (pwr.repository_id IS NULL OR pwr.repository_id=workflow_runs.repository_id)
           )",
        params![drift.repository_id, drift.workflow_name, drift.project_id],
    )?;
    Ok(())
}

fn add_dynamic_rule_for_resolution(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO dynamic_workflow_rules(
           project_id,repository_id,workflow_name,active,protected,created_at
         ) VALUES(?,?,?,1,1,?)",
        params![
            drift.project_id,
            drift.repository_id,
            drift.workflow_name,
            now
        ],
    )?;
    conn.execute(
        "UPDATE dynamic_workflow_rules SET active=1,protected=1
         WHERE project_id=? AND repository_id=? AND workflow_name=?",
        params![drift.project_id, drift.repository_id, drift.workflow_name],
    )?;
    Ok(())
}

fn remove_dynamic_rule_for_resolution(
    conn: &Connection,
    drift: &ResponsibilityMapDrift,
) -> Result<()> {
    let deleted = conn.execute(
        "DELETE FROM dynamic_workflow_rules
         WHERE project_id=? AND repository_id=? AND workflow_name=?",
        params![drift.project_id, drift.repository_id, drift.workflow_name],
    )?;
    if deleted == 0 {
        return Err(anyhow!(
            "제거할 repository-scoped Dynamic 규칙을 찾지 못했습니다."
        ));
    }
    Ok(())
}

#[tauri::command]
fn get_responsibility_resolution_preview(
    input: ResponsibilityResolutionPreviewInput,
    state: State<'_, AppState>,
) -> std::result::Result<ResponsibilityResolutionPreview, String> {
    let result = (|| -> Result<ResponsibilityResolutionPreview> {
        let conn = db(&state)?;
        let drift = current_responsibility_drift(&conn, &input.review_key)?;
        resolution_preview_for_drift(&conn, &drift)
    })();
    result.map_err(|e| e.to_string())
}

fn defer_responsibility_drift_with_conn(
    conn: &Connection,
    input: &DeferResponsibilityDriftInput,
) -> Result<ResponsibilityResolutionResult> {
    let drift = current_responsibility_drift(conn, &input.review_key)?;
    let current_binding = current_watchtower_responsibility_binding(conn, &drift)?;
    if drift.fingerprint != input.fingerprint {
        let audit_id = insert_resolution_audit(
            conn,
            &drift,
            "defer",
            &input.fingerprint,
            &drift.fingerprint,
            current_binding.as_deref(),
            "stale_rejected",
        )?;
        return Ok(ResponsibilityResolutionResult {
            status: "stale_rejected".into(),
            audit_id,
            current_drift: Some(drift),
        });
    }
    let audit_id = insert_resolution_audit(
        conn,
        &drift,
        "defer",
        &input.fingerprint,
        &drift.fingerprint,
        current_binding.as_deref(),
        "deferred",
    )?;
    let mut current = drift.clone();
    current.review_status = "deferred".into();
    Ok(ResponsibilityResolutionResult {
        status: "deferred".into(),
        audit_id,
        current_drift: Some(current),
    })
}

fn reopen_responsibility_drift_with_conn(
    conn: &Connection,
    input: &DeferResponsibilityDriftInput,
) -> Result<ResponsibilityResolutionResult> {
    let drift = current_responsibility_drift(conn, &input.review_key)?;
    let current_binding = current_watchtower_responsibility_binding(conn, &drift)?;
    if drift.fingerprint != input.fingerprint {
        let audit_id = insert_resolution_audit(
            conn,
            &drift,
            "reopen",
            &input.fingerprint,
            &drift.fingerprint,
            current_binding.as_deref(),
            "stale_rejected",
        )?;
        return Ok(ResponsibilityResolutionResult {
            status: "stale_rejected".into(),
            audit_id,
            current_drift: Some(drift),
        });
    }
    let audit_id = insert_resolution_audit(
        conn,
        &drift,
        "reopen",
        &input.fingerprint,
        &drift.fingerprint,
        current_binding.as_deref(),
        "still_open",
    )?;
    let mut current = drift.clone();
    current.review_status = "open".into();
    Ok(ResponsibilityResolutionResult {
        status: "open".into(),
        audit_id,
        current_drift: Some(current),
    })
}

fn resolve_responsibility_drift_with_conn(
    conn: &Connection,
    input: &ResolveResponsibilityDriftInput,
) -> Result<ResponsibilityResolutionResult> {
    let drift = current_responsibility_drift(conn, &input.review_key)?;
    let current_binding = current_watchtower_responsibility_binding(conn, &drift)?;
    if drift.fingerprint != input.fingerprint {
        let audit_id = insert_resolution_audit(
            conn,
            &drift,
            &input.action,
            &input.fingerprint,
            &drift.fingerprint,
            current_binding.as_deref(),
            "stale_rejected",
        )?;
        return Ok(ResponsibilityResolutionResult {
            status: "stale_rejected".into(),
            audit_id,
            current_drift: Some(drift),
        });
    }
    let preview = resolution_preview_for_drift(conn, &drift)?;
    if !preview.executable || preview.action != input.action {
        let audit_id = insert_resolution_audit(
            conn,
            &drift,
            &input.action,
            &input.fingerprint,
            &drift.fingerprint,
            current_binding.as_deref(),
            "blocked",
        )?;
        return Ok(ResponsibilityResolutionResult {
            status: "blocked".into(),
            audit_id,
            current_drift: Some(drift),
        });
    }

    let tx = conn.unchecked_transaction()?;
    let mutation_result: Result<()> = (|| {
        match input.action.as_str() {
            "add_project_wide_rule" => add_project_rule_for_resolution(&tx, &drift)?,
            "add_dynamic_rule" => add_dynamic_rule_for_resolution(&tx, &drift)?,
            "reclassify_to_project_wide" => {
                remove_dynamic_rule_for_resolution(&tx, &drift)?;
                add_project_rule_for_resolution(&tx, &drift)?;
            }
            "reclassify_to_dynamic" => {
                remove_project_rule_for_resolution(&tx, &drift)?;
                add_dynamic_rule_for_resolution(&tx, &drift)?;
            }
            "remove_stale_rule" => match drift.watchtower_binding.as_deref() {
                Some("project-wide") => remove_project_rule_for_resolution(&tx, &drift)?,
                Some("dynamic") => remove_dynamic_rule_for_resolution(&tx, &drift)?,
                _ => {
                    return Err(anyhow!(
                        "제거할 stale responsibility 규칙을 판정하지 못했습니다."
                    ))
                }
            },
            _ => {
                return Err(anyhow!(
                    "지원하지 않는 Responsibility resolution action입니다."
                ))
            }
        }
        Ok(())
    })();
    if let Err(error) = mutation_result {
        tx.rollback()?;
        let resulting_binding = current_watchtower_responsibility_binding(conn, &drift)?;
        let audit_id = insert_resolution_audit(
            conn,
            &drift,
            &input.action,
            &input.fingerprint,
            &drift.fingerprint,
            resulting_binding.as_deref(),
            "failed",
        )?;
        return Err(anyhow!(
            "Responsibility resolution 실패 (audit #{audit_id}): {error}"
        ));
    }

    let resulting_binding = current_watchtower_responsibility_binding(&tx, &drift)?;
    let after = responsibility_map_drifts(&tx)?
        .into_iter()
        .find(|item| item.review_key == drift.review_key);
    let result_status = if after.is_none() {
        "resolved"
    } else {
        "still_open"
    };
    let audit_id = insert_resolution_audit(
        &tx,
        &drift,
        &input.action,
        &input.fingerprint,
        &drift.fingerprint,
        resulting_binding.as_deref(),
        result_status,
    )?;
    tx.commit()?;
    Ok(ResponsibilityResolutionResult {
        status: result_status.into(),
        audit_id,
        current_drift: after,
    })
}

#[tauri::command]
fn defer_responsibility_drift(
    input: DeferResponsibilityDriftInput,
    state: State<'_, AppState>,
) -> std::result::Result<ResponsibilityResolutionResult, String> {
    let result = (|| -> Result<ResponsibilityResolutionResult> {
        let conn = db(&state)?;
        defer_responsibility_drift_with_conn(&conn, &input)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn reopen_responsibility_drift(
    input: DeferResponsibilityDriftInput,
    state: State<'_, AppState>,
) -> std::result::Result<ResponsibilityResolutionResult, String> {
    let result = (|| -> Result<ResponsibilityResolutionResult> {
        let conn = db(&state)?;
        reopen_responsibility_drift_with_conn(&conn, &input)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn resolve_responsibility_drift(
    input: ResolveResponsibilityDriftInput,
    state: State<'_, AppState>,
) -> std::result::Result<ResponsibilityResolutionResult, String> {
    let result = (|| -> Result<ResponsibilityResolutionResult> {
        let conn = db(&state)?;
        resolve_responsibility_drift_with_conn(&conn, &input)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_responsibility_resolution_history(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<ResponsibilityResolutionAuditEntry>, String> {
    let result = (|| -> Result<Vec<ResponsibilityResolutionAuditEntry>> {
        let conn = db(&state)?;
        responsibility_resolution_history(&conn, 200)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_run_attribution(
    run_id: i64,
    state: State<'_, AppState>,
) -> std::result::Result<RunAttributionDetail, String> {
    let conn = db(&state).map_err(|e| e.to_string())?;
    load_run_attribution_detail(&conn, run_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn poll_now(app: AppHandle) -> std::result::Result<Dashboard, String> {
    poll_all(&app).await.map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    build_dashboard(&state).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_project(
    input: ProjectInput,
    state: State<'_, AppState>,
) -> std::result::Result<i64, String> {
    let result = (|| -> Result<i64> {
        let name = input.name.trim();
        let project_key = input.project_key.trim().to_lowercase();
        if name.is_empty() {
            return Err(anyhow!("프로젝트 이름을 입력하십시오."));
        }
        validate_track_key(&project_key)?;
        let conn = db(&state)?;
        let now = Utc::now().to_rfc3339();
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE projects SET name=?,project_key=?,active=1,updated_at=? WHERE id=?",
                params![name, project_key, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 프로젝트를 찾지 못했습니다."));
            }
            id
        } else {
            conn.execute(
                "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES(?,?,1,?,?)",
                params![name, project_key, now, now],
            )?;
            conn.last_insert_rowid()
        };
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_project(id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let repository_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM monitored_repositories WHERE project_id=?",
            params![id],
            |row| row.get(0),
        )?;
        let track_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM watch_tracks WHERE project_id=?",
            params![id],
            |row| row.get(0),
        )?;
        if repository_count > 0 || track_count > 0 {
            return Err(anyhow!(
                "프로젝트에 저장소 또는 트랙이 남아 있어 삭제할 수 없습니다."
            ));
        }
        let deleted = conn.execute("DELETE FROM projects WHERE id=?", params![id])?;
        if deleted == 0 {
            return Err(anyhow!("삭제할 프로젝트를 찾지 못했습니다."));
        }
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_project_workflow_rule(
    input: ProjectWorkflowRuleInput,
    state: State<'_, AppState>,
) -> std::result::Result<i64, String> {
    let result = (|| -> Result<i64> {
        let workflow_name = input.workflow_name.trim();
        if workflow_name.is_empty() {
            return Err(anyhow!("Workflow 이름을 입력하십시오."));
        }
        let conn = db(&state)?;
        if let Some(repository_id) = input.repository_id {
            let belongs: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM monitored_repositories WHERE id=? AND project_id=?)",
                params![repository_id, input.project_id],
                |row| Ok(row.get::<_, i64>(0)? != 0),
            )?;
            if !belongs {
                return Err(anyhow!("저장소가 선택한 프로젝트에 속하지 않습니다."));
            }
        }
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
             VALUES(?,?,?,1,?)",
            params![input.project_id, input.repository_id, workflow_name, now],
        )?;
        conn.execute(
            "UPDATE project_workflow_rules SET active=1
             WHERE project_id=? AND workflow_name=?
               AND ((repository_id IS NULL AND ? IS NULL) OR repository_id=?)",
            params![
                input.project_id,
                workflow_name,
                input.repository_id,
                input.repository_id
            ],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM project_workflow_rules
             WHERE project_id=? AND workflow_name=? AND ((repository_id IS NULL AND ? IS NULL) OR repository_id=?)",
            params![input.project_id, workflow_name, input.repository_id, input.repository_id],
            |row| row.get(0),
        )?;

        conn.execute(
            "DELETE FROM run_assignments
             WHERE manual=0 AND run_id IN (
               SELECT wr.run_id FROM workflow_runs wr
               JOIN monitored_repositories mr ON mr.id=wr.repository_id
               WHERE mr.project_id=? AND wr.workflow_name=?
                 AND (? IS NULL OR wr.repository_id=?)
             )",
            params![
                input.project_id,
                workflow_name,
                input.repository_id,
                input.repository_id
            ],
        )?;
        conn.execute(
            "UPDATE workflow_runs
             SET resolution_status='project'
             WHERE ignored=0
               AND workflow_name=?
               AND repository_id IN (
                 SELECT id FROM monitored_repositories
                 WHERE project_id=? AND (? IS NULL OR id=?)
               )
               AND NOT EXISTS(
                 SELECT 1 FROM run_assignments ra
                 WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
               )",
            params![
                workflow_name,
                input.project_id,
                input.repository_id,
                input.repository_id
            ],
        )?;
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_project_workflow_rule(
    id: i64,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let rule: Option<(i64, Option<i64>, String)> = conn
            .query_row(
                "SELECT project_id,repository_id,workflow_name FROM project_workflow_rules WHERE id=?",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((project_id, repository_id, workflow_name)) = rule else {
            return Err(anyhow!("삭제할 공용 CI 규칙을 찾지 못했습니다."));
        };
        conn.execute("DELETE FROM project_workflow_rules WHERE id=?", params![id])?;
        conn.execute(
            "UPDATE workflow_runs
             SET resolution_status='unassigned'
             WHERE ignored=0
               AND resolution_status='project'
               AND workflow_name=?
               AND repository_id IN (
                 SELECT mr.id FROM monitored_repositories mr
                 WHERE mr.project_id=? AND (? IS NULL OR mr.id=?)
               )
               AND NOT EXISTS(
                 SELECT 1 FROM project_workflow_rules pwr
                 JOIN monitored_repositories mr2 ON mr2.id=workflow_runs.repository_id
                 WHERE pwr.active=1
                   AND pwr.project_id=mr2.project_id
                   AND pwr.workflow_name=workflow_runs.workflow_name
                   AND (pwr.repository_id IS NULL OR pwr.repository_id=workflow_runs.repository_id)
               )",
            params![workflow_name, project_id, repository_id, repository_id],
        )?;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_dynamic_workflow_rule(
    input: DynamicWorkflowRuleInput,
    state: State<'_, AppState>,
) -> std::result::Result<i64, String> {
    let result = (|| -> Result<i64> {
        let workflow_name = input.workflow_name.trim();
        if workflow_name.is_empty() {
            return Err(anyhow!("Workflow 이름을 입력하십시오."));
        }
        let conn = db(&state)?;
        let project_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=? AND active=1)",
            params![input.project_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )?;
        if !project_exists {
            return Err(anyhow!("프로젝트를 찾지 못했습니다."));
        }
        if let Some(repository_id) = input.repository_id {
            let belongs: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM monitored_repositories WHERE id=? AND project_id=?)",
                params![repository_id, input.project_id],
                |row| Ok(row.get::<_, i64>(0)? != 0),
            )?;
            if !belongs {
                return Err(anyhow!("저장소가 선택한 프로젝트에 속하지 않습니다."));
            }
        }
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,?,1,0,?)",
            params![input.project_id, input.repository_id, workflow_name, now],
        )?;
        conn.execute(
            "UPDATE dynamic_workflow_rules SET active=1
             WHERE project_id=? AND workflow_name=?
               AND ((repository_id IS NULL AND ? IS NULL) OR repository_id=?)",
            params![
                input.project_id,
                workflow_name,
                input.repository_id,
                input.repository_id
            ],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM dynamic_workflow_rules
             WHERE project_id=? AND workflow_name=?
               AND ((repository_id IS NULL AND ? IS NULL) OR repository_id=?)",
            params![
                input.project_id,
                workflow_name,
                input.repository_id,
                input.repository_id
            ],
            |row| row.get(0),
        )?;
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_dynamic_workflow_rule(
    id: i64,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let protected: Option<i64> = conn
            .query_row(
                "SELECT protected FROM dynamic_workflow_rules WHERE id=?",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(protected) = protected else {
            return Err(anyhow!("삭제할 Dynamic Workflow 규칙을 찾지 못했습니다."));
        };
        if protected != 0 {
            return Err(anyhow!("기본 Dynamic Workflow 계약은 삭제할 수 없습니다."));
        }
        conn.execute("DELETE FROM dynamic_workflow_rules WHERE id=?", params![id])?;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_track(input: TrackInput, state: State<'_, AppState>) -> std::result::Result<i64, String> {
    let result = (|| -> Result<i64> {
        let name = input.name.trim();
        let track_key = input.track_key.trim().to_lowercase();
        if name.is_empty() {
            return Err(anyhow!("트랙 이름을 입력하십시오."));
        }
        validate_track_key(&track_key)?;
        if input.long_ci_minutes <= 0 || input.long_ci_minutes > 10080 {
            return Err(anyhow!("장기 CI 기준시간은 1~10080분 사이로 입력하십시오."));
        }
        let conn = db(&state)?;
        let project_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=? AND active=1)",
            params![input.project_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )?;
        if !project_exists {
            return Err(anyhow!("프로젝트를 찾지 못했습니다."));
        }
        let now = Utc::now().to_rfc3339();
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE watch_tracks SET project_id=?,name=?,track_key=?,long_ci_minutes=?,active=1,updated_at=? WHERE id=?",
                params![input.project_id, name, track_key, input.long_ci_minutes, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 트랙을 찾지 못했습니다."));
            }
            id
        } else {
            conn.execute(
                "INSERT INTO watch_tracks(project_id,name,track_key,long_ci_minutes,active,created_at,updated_at) VALUES(?,?,?,?,1,?,?)",
                params![input.project_id, name, track_key, input.long_ci_minutes, now, now],
            )?;
            conn.last_insert_rowid()
        };
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_track(id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let mut conn = db(&state)?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE workflow_runs
             SET resolution_status='unassigned'
             WHERE run_id IN (SELECT run_id FROM run_assignments WHERE track_id=?)",
            params![id],
        )?;
        let deleted = tx.execute("DELETE FROM watch_tracks WHERE id=?", params![id])?;
        if deleted == 0 {
            return Err(anyhow!("삭제할 트랙을 찾지 못했습니다."));
        }
        tx.commit()?;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_repository(
    input: RepositoryInput,
    state: State<'_, AppState>,
) -> std::result::Result<i64, String> {
    let result = (|| -> Result<i64> {
        let repo = input.repo.trim();
        validate_repo(repo)?;
        let conn = db(&state)?;
        let project_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=? AND active=1)",
            params![input.project_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )?;
        if !project_exists {
            return Err(anyhow!("프로젝트를 찾지 못했습니다."));
        }
        let now = Utc::now().to_rfc3339();
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE monitored_repositories SET project_id=?,repo=?,enabled=?,updated_at=? WHERE id=?",
                params![input.project_id, repo, if input.enabled { 1 } else { 0 }, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 저장소를 찾지 못했습니다."));
            }
            id
        } else {
            conn.execute(
                "INSERT INTO monitored_repositories(project_id,repo,enabled,created_at,updated_at) VALUES(?,?,?,?,?)",
                params![input.project_id, repo, if input.enabled { 1 } else { 0 }, now, now],
            )?;
            conn.last_insert_rowid()
        };
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_repository(id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
    let conn = db(&state).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM monitored_repositories WHERE id=?", params![id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn assign_run_to_project_in_conn(
    conn: &Connection,
    run_id: i64,
    project_id: i64,
    learn_rule: bool,
    now: &str,
) -> Result<()> {
    let (repository_project_id, workflow_name): (i64, String) = conn.query_row(
        "SELECT mr.project_id,wr.workflow_name
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         WHERE wr.run_id=?",
        params![run_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if repository_project_id != project_id {
        return Err(anyhow!("Run과 프로젝트가 일치하지 않습니다."));
    }

    conn.execute(
        "DELETE FROM run_assignments WHERE run_id=?",
        params![run_id],
    )?;
    conn.execute(
        "UPDATE workflow_runs SET resolution_status='project',ignored=0 WHERE run_id=?",
        params![run_id],
    )?;

    if learn_rule {
        conn.execute(
            "INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
             VALUES(?,NULL,?,1,?)",
            params![project_id, workflow_name, now],
        )?;
        conn.execute(
            "UPDATE project_workflow_rules SET active=1
             WHERE project_id=? AND repository_id IS NULL AND workflow_name=?",
            params![project_id, workflow_name],
        )?;
        conn.execute(
            "DELETE FROM run_assignments
             WHERE manual=0 AND run_id IN (
               SELECT wr.run_id FROM workflow_runs wr
               JOIN monitored_repositories mr ON mr.id=wr.repository_id
               WHERE mr.project_id=? AND wr.workflow_name=?
             )",
            params![project_id, workflow_name],
        )?;
        conn.execute(
            "UPDATE workflow_runs
             SET resolution_status='project'
             WHERE ignored=0 AND workflow_name=?
               AND repository_id IN (SELECT id FROM monitored_repositories WHERE project_id=?)
               AND NOT EXISTS(
                 SELECT 1 FROM run_assignments ra
                 WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
               )",
            params![workflow_name, project_id],
        )?;
    }

    Ok(())
}

#[tauri::command]
fn assign_run_to_project(
    run_id: i64,
    project_id: i64,
    learn_rule: bool,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let now = Utc::now().to_rfc3339();
        assign_run_to_project_in_conn(&conn, run_id, project_id, learn_rule, &now)
    })();
    result.map_err(|e| e.to_string())
}

fn assign_run_in_conn(conn: &Connection, run_id: i64, track_id: i64, now: &str) -> Result<()> {
    let (workflow_name, workflow_path, run_project_id, track_project_id): (
        String,
        Option<String>,
        i64,
        i64,
    ) = conn.query_row(
        "SELECT wr.workflow_name,wr.workflow_path,mr.project_id,wt.project_id
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN watch_tracks wt ON wt.id=?
         WHERE wr.run_id=?",
        params![track_id, run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if run_project_id != track_project_id {
        return Err(anyhow!(
            "다른 프로젝트의 트랙에는 Run을 귀속할 수 없습니다."
        ));
    }

    conn.execute(
        "INSERT INTO run_assignments(run_id,track_id,confidence,source,reason,manual,assigned_at)
         VALUES(?,?,100,'manual','사용자 수동 귀속',1,?)
         ON CONFLICT(run_id) DO UPDATE SET track_id=excluded.track_id,confidence=100,source='manual',reason='사용자 수동 귀속',manual=1,assigned_at=excluded.assigned_at",
        params![run_id, track_id, now],
    )?;
    conn.execute(
        "UPDATE workflow_runs SET resolution_status='assigned',ignored=0 WHERE run_id=?",
        params![run_id],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO track_fingerprints(track_id,signal_type,pattern,repository_id,weight,learned_from_run_id,active,created_at)
         SELECT ?, 'workflow_name', ?, wr.repository_id, 50, ?, 1, ?
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN watch_tracks wt ON wt.id=?
         WHERE wr.run_id=? AND mr.project_id=wt.project_id",
        params![track_id, workflow_name, run_id, now, track_id, run_id],
    )?;

    if let Some(path) = workflow_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        conn.execute(
            "INSERT OR IGNORE INTO track_fingerprints(track_id,signal_type,pattern,repository_id,weight,learned_from_run_id,active,created_at)
             SELECT ?, 'workflow_path', ?, wr.repository_id, 35, ?, 1, ?
             FROM workflow_runs wr
             JOIN monitored_repositories mr ON mr.id=wr.repository_id
             JOIN watch_tracks wt ON wt.id=?
             WHERE wr.run_id=? AND mr.project_id=wt.project_id",
            params![track_id, path, run_id, now, track_id, run_id],
        )?;
    }

    Ok(())
}

#[tauri::command]
fn assign_run(
    run_id: i64,
    track_id: i64,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let now = Utc::now().to_rfc3339();
        assign_run_in_conn(&conn, run_id, track_id, &now)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn ignore_run(run_id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
    let conn = db(&state).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE workflow_runs SET ignored=1,resolution_status='ignored' WHERE run_id=?",
        params![run_id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(
    settings: Settings,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    if settings.queue_congestion_threshold < 1 || settings.queue_congestion_threshold > 500 {
        return Err("Queue 기준은 1~500 사이여야 합니다.".into());
    }
    if settings.active_poll_seconds < 10 || settings.active_poll_seconds > 3600 {
        return Err("활성 Polling은 10~3600초 사이여야 합니다.".into());
    }
    if settings.idle_poll_seconds < 30 || settings.idle_poll_seconds > 7200 {
        return Err("유휴 Polling은 30~7200초 사이여야 합니다.".into());
    }
    let conn = db(&state).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE app_settings SET queue_congestion_threshold=?,active_poll_seconds=?,idle_poll_seconds=?,auto_archive_completed=? WHERE id=1",
        params![
            settings.queue_congestion_threshold,
            settings.active_poll_seconds,
            settings.idle_poll_seconds,
            if settings.auto_archive_completed { 1 } else { 0 }
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_responsibility_review_policy(
    policy: ResponsibilityReviewPolicy,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let targets = [
        policy.p0_target_hours,
        policy.p1_target_hours,
        policy.p2_target_hours,
    ];
    if targets.iter().any(|value| *value < 1 || *value > 720) {
        return Err("Responsibility SLA target은 1~720시간 사이여야 합니다.".into());
    }
    if policy.p1_target_hours <= policy.p2_target_hours {
        return Err("P1 SLA target은 P2 SLA target보다 커야 합니다.".into());
    }
    for (due, target) in [
        (policy.p0_due_soon_hours, policy.p0_target_hours),
        (policy.p1_due_soon_hours, policy.p1_target_hours),
        (policy.p2_due_soon_hours, policy.p2_target_hours),
    ] {
        if due < 1 || due > target {
            return Err("Due-soon 기준은 1시간 이상이며 해당 SLA target 이하여야 합니다.".into());
        }
    }
    let conn = db(&state).map_err(|e| e.to_string())?;
    let project_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=? AND active=1)",
            params![policy.project_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )
        .map_err(|e| e.to_string())?;
    if !project_exists {
        return Err("활성 Project를 찾을 수 없습니다.".into());
    }
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO responsibility_review_policies(
           project_id,p0_target_hours,p1_target_hours,p2_target_hours,
           p0_due_soon_hours,p1_due_soon_hours,p2_due_soon_hours,
           notify_warning,notify_critical,updated_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?)
         ON CONFLICT(project_id) DO UPDATE SET
           p0_target_hours=excluded.p0_target_hours,
           p1_target_hours=excluded.p1_target_hours,
           p2_target_hours=excluded.p2_target_hours,
           p0_due_soon_hours=excluded.p0_due_soon_hours,
           p1_due_soon_hours=excluded.p1_due_soon_hours,
           p2_due_soon_hours=excluded.p2_due_soon_hours,
           notify_warning=excluded.notify_warning,
           notify_critical=excluded.notify_critical,
           updated_at=excluded.updated_at",
        params![
            policy.project_id,
            policy.p0_target_hours,
            policy.p1_target_hours,
            policy.p2_target_hours,
            policy.p0_due_soon_hours,
            policy.p1_due_soon_hours,
            policy.p2_due_soon_hours,
            if policy.notify_warning { 1 } else { 0 },
            if policy.notify_critical { 1 } else { 0 },
            now,
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_github_token(token: String) -> std::result::Result<(), String> {
    let token = token.trim();
    if token.len() < 20 {
        return Err("GitHub PAT 형식이 올바르지 않습니다.".into());
    }
    let entry = keyring_entry().map_err(|e| e.to_string())?;
    entry.set_password(token).map_err(|e| e.to_string())?;
    let verified = entry.get_password().map_err(|e| e.to_string())?;
    if verified.trim() != token {
        return Err("PAT 저장 후 검증에 실패했습니다.".into());
    }
    Ok(())
}

#[tauri::command]
fn clear_github_token() -> std::result::Result<(), String> {
    match keyring_entry() {
        Ok(entry) => {
            let _ = entry.delete_credential();
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

#[tauri::command]
fn open_external(url: String) -> std::result::Result<(), String> {
    if !url.starts_with("https://github.com/") {
        return Err("GitHub URL만 열 수 있습니다.".into());
    }
    open::that(url).map_err(|e| e.to_string())
}

fn install_tray(app: &tauri::App) -> Result<()> {
    let show = MenuItem::with_id(app, "show", "CI Watchtower 열기", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("CI Watchtower")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn install_close_to_tray(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let window_for_event = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_for_event.hide();
            }
        });
    }
}

fn start_poller(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        loop {
            let _ = poll_all(&handle).await;
            let seconds = {
                let state = handle.state::<AppState>();
                match build_dashboard(&state) {
                    Ok(dashboard) => {
                        let active = dashboard.running_count > 0
                            || dashboard.queued_count > 0
                            || dashboard.unassigned_count > 0;
                        if active {
                            dashboard.settings.active_poll_seconds
                        } else {
                            dashboard.settings.idle_poll_seconds
                        }
                    }
                    Err(_) => DEFAULT_IDLE_POLL_SECONDS,
                }
            };
            tokio::time::sleep(Duration::from_secs(seconds.max(10) as u64)).await;
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let db_path = app.path().app_data_dir()?.join("ci-watchtower.sqlite3");
            init_db(&db_path)?;
            app.manage(AppState {
                db_path,
                poll_in_flight: AtomicBool::new(false),
            });
            install_tray(app)?;
            install_close_to_tray(app);
            start_poller(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            get_responsibility_resolution_preview,
            resolve_responsibility_drift,
            defer_responsibility_drift,
            reopen_responsibility_drift,
            get_responsibility_resolution_history,
            get_run_attribution,
            poll_now,
            save_project,
            delete_project,
            save_project_workflow_rule,
            delete_project_workflow_rule,
            save_dynamic_workflow_rule,
            delete_dynamic_workflow_rule,
            save_track,
            delete_track,
            save_repository,
            delete_repository,
            assign_run_to_project,
            assign_run,
            ignore_run,
            save_settings,
            save_responsibility_review_policy,
            set_github_token,
            clear_github_token,
            open_external
        ])
        .run(tauri::generate_context!())
        .expect("error while running CI Watchtower");
}

#[cfg(test)]
mod tests;
