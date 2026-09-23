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
    repositories: Vec<MonitoredRepository>,
    tracks: Vec<DashboardTrack>,
    project_workflow_rules: Vec<ProjectWorkflowRule>,
    repository_scope_stats: Vec<RepositoryScopeStats>,
    producer_contract_stats: Vec<ProducerContractStats>,
    producer_contract_runs: Vec<ProducerContractRun>,
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

    for workflow_name in [
        "BEJEWELY Current Main Health",
        "PIE Prospective Shadow",
        "BEJEWELY Security Boundary",
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,NULL,?,1,?)",
            params![project_id, workflow_name, now],
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

    let project_count: i64 = conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
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
        conn.query_row("SELECT id FROM projects WHERE active=1 ORDER BY id LIMIT 1", [], |row| row.get(0))?
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
        for workflow_name in ["CI", "Governance", "Web PR Domain Gates", "PIE Prospective Shadow"] {
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
            ] {
                conn.execute(
                    "INSERT OR IGNORE INTO project_workflow_rules(
                       project_id,repository_id,workflow_name,active,created_at
                     ) VALUES(?,?,?,1,?)",
                    params![myeongha_project_id, repository_id, workflow_name, now],
                )?;
            }
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
             ALTER TABLE track_aliases_v04 RENAME TO track_aliases;"
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
         END;"
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
    headers.insert(header::ACCEPT, header::HeaderValue::from_static("application/vnd.github+json"));
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
        .user_agent("ci-watchtower/0.3.7")
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
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
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
        return Err(anyhow!("Track Key는 하이픈으로 시작하거나 끝날 수 없습니다."));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow!("Track Key는 영문 소문자, 숫자, 하이픈만 사용할 수 있습니다."));
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

fn run_summary_from_row(row: &rusqlite::Row<'_>, now: DateTime<Utc>) -> rusqlite::Result<WorkflowRunSummary> {
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
    let rows = stmt.query_map(params![track_id, limit], |row| run_summary_from_row(row, now))?;
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
           SELECT wr.run_id,mr.project_id,mr.id AS repository_id,mr.repo,
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
                  ) AS repository_rank
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
           WHERE wr.ignored=0
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
                END AS contract_compliant
         FROM recent
         WHERE repository_rank<=?
         ORDER BY created_at DESC,run_id DESC",
    )?;
    let rows = stmt.query_map(params![sample_per_repository.max(1)], |row| {
        Ok(ProducerContractRun {
            run: run_summary_from_row(row, now)?,
            bucket: row.get(20)?,
            contract_compliant: row.get::<_, i64>(21)? != 0,
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
    if runs
        .iter()
        .any(|r| matches!(r.status.as_str(), "queued" | "requested" | "pending" | "waiting"))
    {
        return "queued".into();
    }
    let latest = &runs[0];
    if latest.status == "completed" {
        match latest.conclusion.as_deref() {
            Some("success") => "green".into(),
            Some("failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure" | "stale") => "red".into(),
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
    let repositories = list_repositories(&conn, false)?;
    let tracks = list_tracks(&conn, true)?;
    let project_workflow_rules = list_project_workflow_rules(&conn)?;
    let mut dashboard_tracks = Vec::with_capacity(tracks.len());
    for track in tracks {
        let runs = runs_for_track(&conn, track.id, 30)?;
        let health = track_health(&runs);
        let elapsed_seconds = runs
            .iter()
            .filter(|r| matches!(r.status.as_str(), "in_progress" | "queued" | "requested" | "pending" | "waiting"))
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
    let running_count: i64 = repositories.iter().filter(|r| r.enabled).map(|r| r.running_count).sum();
    let queued_count: i64 = repositories.iter().filter(|r| r.enabled).map(|r| r.queued_count).sum();
    let repository_scope_stats = repository_scope_stats(&conn)?;
    let producer_contract_stats =
        producer_contract_stats(&conn, PRODUCER_CONTRACT_SAMPLE_PER_REPOSITORY)?;
    let producer_contract_runs =
        producer_contract_runs(&conn, PRODUCER_CONTRACT_SAMPLE_PER_REPOSITORY)?;
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
        repositories,
        tracks: dashboard_tracks,
        project_workflow_rules,
        repository_scope_stats,
        producer_contract_stats,
        producer_contract_runs,
        project_runs,
        unassigned_runs,
    })
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(client: &Client, url: String, label: &str) -> Result<T> {
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
            let url = format!("https://api.github.com/repos/{repo}/pulls/{}", pr_ref.number);
            if let Ok(pr) = fetch_json::<GithubPull>(client, url, "PR 조회 실패").await {
                pulls.push(pr);
            }
        }
    } else {
        let url = format!("https://api.github.com/repos/{repo}/commits/{}/pulls", run.head_sha);
        if let Ok(found) = fetch_json::<Vec<GithubPull>>(client, url, "Commit PR 조회 실패").await {
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

    let known: HashMap<&str, i64> = tracks.iter().map(|t| (t.track_key.as_str(), t.id)).collect();

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
    let previous: Option<(i64, String, Option<String>, Option<String>, Option<i64>, Option<i64>)> =
        if trigger.is_some() {
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
    )) = previous else {
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

fn notify_runs(app: &AppHandle, conn: &Connection, tracks: &[Track], now: DateTime<Utc>) -> Result<()> {
    let now_str = now.to_rfc3339();
    for track in tracks {
        let runs = runs_for_track(conn, track.id, 100)?;
        for run in runs {
            if run.status == "completed" {
                let event_type = match run.conclusion.as_deref() {
                    Some("success") => "green",
                    Some("failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure" | "stale") => "red",
                    _ => "done",
                };
                if mark_notified(conn, track.id, run.id, run.run_attempt, event_type, &now_str)? {
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
                let running_count = runs.iter().filter(|r| r.status == "in_progress").count() as i64;
                let queued_count = runs
                    .iter()
                    .filter(|r| matches!(r.status.as_str(), "queued" | "requested" | "pending" | "waiting"))
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
                    load_stored_unresolved_runs(
                        &conn,
                        repository.id,
                        HISTORICAL_RECONCILE_BATCH,
                    )?
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
fn save_project(input: ProjectInput, state: State<'_, AppState>) -> std::result::Result<i64, String> {
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
            return Err(anyhow!("프로젝트에 저장소 또는 트랙이 남아 있어 삭제할 수 없습니다."));
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
            params![input.project_id, workflow_name, input.repository_id, input.repository_id],
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
            params![input.project_id, workflow_name, input.repository_id, input.repository_id],
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
            params![workflow_name, input.project_id, input.repository_id, input.repository_id],
        )?;
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_project_workflow_rule(id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
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

    conn.execute("DELETE FROM run_assignments WHERE run_id=?", params![run_id])?;
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
        return Err(anyhow!("다른 프로젝트의 트랙에는 Run을 귀속할 수 없습니다."));
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
            get_run_attribution,
            poll_now,
            save_project,
            delete_project,
            save_project_workflow_rule,
            delete_project_workflow_rule,
            save_track,
            delete_track,
            save_repository,
            delete_repository,
            assign_run_to_project,
            assign_run,
            ignore_run,
            save_settings,
            set_github_token,
            clear_github_token,
            open_external
        ])
        .run(tauri::generate_context!())
        .expect("error while running CI Watchtower");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_v02_db_path(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ci-watchtower-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn seed_legacy_v02_database(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys=ON;

            CREATE TABLE tracks (
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

            CREATE TABLE watch_tracks (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              name TEXT NOT NULL,
              track_key TEXT NOT NULL UNIQUE,
              long_ci_minutes INTEGER NOT NULL CHECK(long_ci_minutes > 0),
              active INTEGER NOT NULL DEFAULT 1,
              legacy_track_id INTEGER UNIQUE,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE monitored_repositories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
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

            CREATE TABLE workflow_runs (
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
              resolution_status TEXT NOT NULL DEFAULT 'unassigned',
              ignored INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE run_attempts (
              run_id INTEGER NOT NULL,
              run_attempt INTEGER NOT NULL,
              status TEXT NOT NULL,
              conclusion TEXT,
              started_at TEXT,
              updated_at TEXT NOT NULL,
              PRIMARY KEY(run_id, run_attempt)
            );

            CREATE TABLE run_assignments (
              run_id INTEGER PRIMARY KEY REFERENCES workflow_runs(run_id) ON DELETE CASCADE,
              track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
              confidence INTEGER NOT NULL,
              source TEXT NOT NULL,
              reason TEXT NOT NULL,
              manual INTEGER NOT NULL DEFAULT 0,
              assigned_at TEXT NOT NULL
            );

            CREATE TABLE run_evidence (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              run_id INTEGER NOT NULL REFERENCES workflow_runs(run_id) ON DELETE CASCADE,
              track_key TEXT NOT NULL,
              signal_type TEXT NOT NULL,
              score INTEGER NOT NULL,
              value TEXT NOT NULL,
              created_at TEXT NOT NULL
            );

            CREATE TABLE track_fingerprints (
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

            CREATE TABLE notifications_v2 (
              track_id INTEGER NOT NULL REFERENCES watch_tracks(id) ON DELETE CASCADE,
              run_id INTEGER NOT NULL,
              run_attempt INTEGER NOT NULL,
              event_type TEXT NOT NULL,
              notified_at TEXT NOT NULL,
              PRIMARY KEY(track_id, run_id, run_attempt, event_type)
            );

            CREATE TABLE app_settings (
              id INTEGER PRIMARY KEY CHECK(id=1),
              queue_congestion_threshold INTEGER NOT NULL,
              active_poll_seconds INTEGER NOT NULL,
              idle_poll_seconds INTEGER NOT NULL,
              auto_archive_completed INTEGER NOT NULL,
              queue_congested INTEGER NOT NULL DEFAULT 0
            );

            INSERT INTO app_settings VALUES(1,4,30,180,0,0);

            INSERT INTO watch_tracks(
              id,name,track_key,long_ci_minutes,active,legacy_track_id,created_at,updated_at
            ) VALUES
              (10,'CI 운영','ops',8,1,NULL,'2026-09-20T00:00:00Z','2026-09-20T00:00:00Z'),
              (20,'결제','commerce',8,1,NULL,'2026-09-20T00:00:00Z','2026-09-20T00:00:00Z');

            INSERT INTO monitored_repositories(
              id,repo,enabled,running_count,queued_count,created_at,updated_at
            ) VALUES
              (100,'gycha0109-beep/MyeongHa',1,0,0,'2026-09-20T00:00:00Z','2026-09-20T00:00:00Z'),
              (101,'gycha0109-beep/Saju',1,0,0,'2026-09-20T00:00:00Z','2026-09-20T00:00:00Z'),
              (200,'gycha0109-beep/K_beauty',1,0,0,'2026-09-20T00:00:00Z','2026-09-20T00:00:00Z');

            INSERT INTO workflow_runs(
              run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
              head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
              created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored
            ) VALUES
              (500,200,1,'Feature CI','.github/workflows/feature.yml','legacy manual','push',
               'ops/legacy','sha500',1,1,'completed','success','https://example/500',
               '2026-09-20T01:00:00Z','2026-09-20T01:00:01Z','2026-09-20T01:01:00Z','2026-09-20T01:01:00Z','assigned',0),
              (501,100,2,'Feature CI','.github/workflows/feature.yml','legacy privacy alias','push',
               'privacy-recovery/fix','sha501',2,1,'completed','success','https://example/501',
               '2026-09-20T02:00:00Z','2026-09-20T02:00:01Z','2026-09-20T02:01:00Z','2026-09-20T02:01:00Z','unassigned',0),
              (502,100,3,'Feature CI','.github/workflows/feature.yml','legacy commerce alias','push',
               'commerce/fix','sha502',3,1,'completed','success','https://example/502',
               '2026-09-20T03:00:00Z','2026-09-20T03:00:01Z','2026-09-20T03:01:00Z','2026-09-20T03:01:00Z','unassigned',0),
              (503,100,4,'CI','.github/workflows/ci.yml','legacy project workflow','push',
               'main','sha503',4,1,'completed','success','https://example/503',
               '2026-09-20T04:00:00Z','2026-09-20T04:00:01Z','2026-09-20T04:01:00Z','2026-09-20T04:01:00Z','assigned',0),
              (504,200,5,'BEJEWELY Current Main Health','.github/workflows/main-health.yml','legacy visualy project workflow','push',
               'main','sha504',5,1,'completed','success','https://example/504',
               '2026-09-20T05:00:00Z','2026-09-20T05:00:01Z','2026-09-20T05:01:00Z','2026-09-20T05:01:00Z','assigned',0);

            INSERT INTO run_assignments(
              run_id,track_id,confidence,source,reason,manual,assigned_at
            ) VALUES
              (500,10,100,'manual','legacy user choice',1,'2026-09-20T01:02:00Z'),
              (503,10,90,'branch','legacy automatic project-wide assignment',0,'2026-09-20T04:02:00Z'),
              (504,10,90,'branch','legacy automatic visualy project-wide assignment',0,'2026-09-20T05:02:00Z');

            INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) VALUES
              (501,'privacy-recovery','branch',90,'privacy-recovery/fix','2026-09-20T02:00:00Z'),
              (502,'commerce','branch',90,'commerce/fix','2026-09-20T03:00:00Z'),
              (500,'ops','branch',90,'ops/legacy','2026-09-20T01:00:00Z');

            INSERT INTO track_fingerprints(
              track_id,signal_type,pattern,repository_id,weight,learned_from_run_id,active,created_at
            ) VALUES
              (10,'workflow_name','Feature CI',100,50,501,1,'2026-09-20T02:00:00Z'),
              (20,'workflow_path','.github/workflows/feature.yml',100,35,502,1,'2026-09-20T03:00:00Z');

            INSERT INTO notifications_v2(track_id,run_id,run_attempt,event_type,notified_at)
              VALUES(10,503,1,'completed','2026-09-20T04:02:00Z');
            "#,
        )
        .unwrap();
    }

    fn track(id: i64, key: &str) -> Track {
        Track {
            id,
            project_id: 1,
            name: key.into(),
            track_key: key.into(),
            long_ci_minutes: 8,
            active: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn extracts_run_name_marker() {
        assert_eq!(
            extract_marker("[WT:frontend-integration] Reader Grounding"),
            Some("frontend-integration".into())
        );
    }

    #[test]
    fn normalizes_legacy_bejewely_taxonomy_key() {
        assert_eq!(
            extract_marker("[WT:taxonomy&AI] Product Query Quality"),
            Some("taxonomy-ai".into())
        );
        assert_eq!(
            extract_track_trailer("feat: x\n\nWatchtower-Track: taxonomy&AI"),
            Some("taxonomy-ai".into())
        );
        assert!(branch_has_key("feat/taxonomy&AI/provider-quality", "taxonomy-ai"));
    }

    #[test]
    fn extracts_pr_or_commit_trailer() {
        assert_eq!(
            extract_track_trailer("feat: x\n\nWatchtower-Track: ops"),
            Some("ops".into())
        );
    }

    #[test]
    fn branch_key_requires_segment_match() {
        assert!(branch_has_key(
            "feat/frontend-integration/reader-scene",
            "frontend-integration"
        ));
        assert!(!branch_has_key("feat/frontend/foo", "frontend-integration"));
    }

    #[test]
    fn track_key_validation_is_bounded() {
        assert!(validate_track_key("frontend-integration").is_ok());
        assert!(validate_track_key("Frontend").is_err());
        assert!(validate_track_key("-ops").is_err());
        assert!(validate_track_key("ops/one").is_err());
    }

    #[test]
    fn legacy_known_tracks_keep_stable_keys() {
        assert_eq!(legacy_track_key("프론트 연동 4", 1), "frontend-integration");
        assert_eq!(legacy_track_key("운영 32", 2), "ops");
        assert_eq!(legacy_track_key("관상 연구 및 검증 2", 3), "face-research");
    }

    fn evidence(key: &str, signal_type: &str, score: i64) -> Evidence {
        Evidence {
            track_key: key.into(),
            signal_type: signal_type.into(),
            score,
            value: signal_type.into(),
        }
    }

    #[test]
    fn unknown_run_name_marker_fails_closed_over_known_branch() {
        let tracks = vec![track(1, "ops")];
        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            vec![
                evidence("unknown-track", "run_name", 100),
                evidence("ops", "branch", 90),
            ],
        );
        assert_eq!(resolution.status, "unassigned");
        assert_eq!(resolution.track_id, None);
        assert_eq!(resolution.confidence, Some(100));
    }

    #[test]
    fn run_name_precedes_pr_marker() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            vec![
                evidence("ops", "run_name", 100),
                evidence("saju", "pr_marker", 98),
            ],
        );
        assert_eq!(resolution.status, "assigned");
        assert_eq!(resolution.track_id, Some(1));
        assert_eq!(resolution.source.as_deref(), Some("run_name"));
    }

    #[test]
    fn pr_marker_precedes_commit_marker() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            vec![
                evidence("ops", "pr_marker", 98),
                evidence("saju", "commit_marker", 96),
            ],
        );
        assert_eq!(resolution.status, "assigned");
        assert_eq!(resolution.track_id, Some(1));
        assert_eq!(resolution.source.as_deref(), Some("pr_marker"));
    }

    #[test]
    fn commit_marker_precedes_branch() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            vec![
                evidence("ops", "commit_marker", 96),
                evidence("saju", "branch", 90),
            ],
        );
        assert_eq!(resolution.status, "assigned");
        assert_eq!(resolution.track_id, Some(1));
        assert_eq!(resolution.source.as_deref(), Some("commit_marker"));
    }

    #[test]
    fn same_priority_pr_markers_conflict() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            vec![
                evidence("ops", "pr_marker", 98),
                evidence("saju", "pr_marker", 98),
            ],
        );
        assert_eq!(resolution.status, "conflict");
        assert_eq!(resolution.track_id, None);
        assert_eq!(resolution.source.as_deref(), Some("explicit_conflict"));
    }

    #[test]
    fn historical_alias_is_canonicalized_before_precedence() {
        let tracks = vec![track(1, "ops")];
        let aliases = HashMap::from([("privacy-recovery".into(), "ops".into())]);
        let resolution = resolve_evidence(
            &tracks,
            &aliases,
            vec![evidence("privacy-recovery", "run_name", 100)],
        );
        assert_eq!(resolution.status, "assigned");
        assert_eq!(resolution.track_id, Some(1));
        assert_eq!(resolution.confidence, Some(100));
    }

    #[test]
    fn repository_scope_stats_keep_project_and_repository_counts_exact() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               ignored INTEGER NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL
             );
             INSERT INTO monitored_repositories(id,project_id)
               VALUES(10,1),(20,1),(30,2);
             INSERT INTO workflow_runs(run_id,repository_id,resolution_status,ignored)
               VALUES(101,10,'unassigned',0),
                     (102,10,'conflict',0),
                     (103,10,'project',0),
                     (104,10,'unassigned',1),
                     (201,20,'unassigned',0),
                     (202,20,'project',0),
                     (301,30,'project',0),
                     (302,30,'unassigned',0);
             INSERT INTO run_assignments(run_id,track_id) VALUES(102,999);"
        ).unwrap();

        let stats = repository_scope_stats(&conn).unwrap();
        let by_repo: HashMap<i64, (i64, i64, i64)> = stats
            .into_iter()
            .map(|item| (
                item.repository_id,
                (item.project_id, item.unassigned_count, item.project_run_count),
            ))
            .collect();

        assert_eq!(by_repo.get(&10), Some(&(1, 1, 1)));
        assert_eq!(by_repo.get(&20), Some(&(1, 1, 1)));
        assert_eq!(by_repo.get(&30), Some(&(2, 1, 1)));
    }

    #[test]
    fn alias_audit_evidence_does_not_double_inference_score() {
        let tracks = vec![track(1, "ops")];
        let aliases = HashMap::from([("privacy-recovery".into(), "ops".into())]);
        let resolution = resolve_evidence(
            &tracks,
            &aliases,
            vec![evidence("privacy-recovery", "workflow_name", 40)],
        );

        assert_eq!(resolution.status, "unassigned");
        assert_eq!(resolution.track_id, None);
        assert_eq!(resolution.confidence, Some(40));
        assert!(resolution
            .evidence
            .iter()
            .any(|item| item.signal_type == "workflow_name_alias"));
    }

    #[test]
    fn independent_alias_signals_can_still_reach_inference_threshold() {
        let tracks = vec![track(1, "ops")];
        let aliases = HashMap::from([("privacy-recovery".into(), "ops".into())]);
        let resolution = resolve_evidence(
            &tracks,
            &aliases,
            vec![
                evidence("privacy-recovery", "workflow_name", 40),
                evidence("privacy-recovery", "workflow_path", 35),
            ],
        );

        assert_eq!(resolution.status, "assigned");
        assert_eq!(resolution.track_id, Some(1));
        assert_eq!(resolution.confidence, Some(75));
        assert_eq!(resolution.source.as_deref(), Some("inference"));
    }

    #[test]
    fn persist_resolution_records_attempt_time_for_unassigned_run() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             INSERT INTO workflow_runs(run_id,resolution_status,last_resolution_attempt_at)
               VALUES(500,'unassigned',NULL);"
        ).unwrap();

        let resolution = Resolution {
            status: "unassigned".into(),
            track_id: None,
            confidence: Some(40),
            source: None,
            reason: Some("insufficient".into()),
            evidence: vec![evidence("ops", "workflow_name", 40)],
        };
        persist_resolution(&conn, 500, &resolution, "2026-09-24T01:00:00Z").unwrap();

        let row: (String, Option<String>) = conn.query_row(
            "SELECT resolution_status,last_resolution_attempt_at
             FROM workflow_runs WHERE run_id=500",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(row.0, "unassigned");
        assert_eq!(row.1.as_deref(), Some("2026-09-24T01:00:00Z"));
    }

    #[test]
    fn historical_reconciliation_records_decision_transition() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               track_key TEXT NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE resolution_reconciliation_audit(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
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
             INSERT INTO workflow_runs VALUES(500,100,'unassigned',NULL);
             INSERT INTO watch_tracks VALUES(7,'ops');
             INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at)
               VALUES(500,'ops','workflow_name',50,'CI','2026-09-24T00:00:00Z');"
        ).unwrap();

        let resolution = Resolution {
            status: "assigned".into(),
            track_id: Some(7),
            confidence: Some(100),
            source: Some("run_name".into()),
            reason: Some("run_name → ops".into()),
            evidence: vec![evidence("ops", "run_name", 100)],
        };
        persist_resolution_with_trigger(
            &conn,
            500,
            &resolution,
            "2026-09-24T02:00:00Z",
            Some("historical_reconcile"),
        ).unwrap();

        let row: (String, String, Option<String>, Option<String>, String) = conn.query_row(
            "SELECT trigger,from_status,to_track_key,to_source,evidence_json
             FROM resolution_reconciliation_audit
             WHERE run_id=500",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
        assert_eq!(row.0, "historical_reconcile");
        assert_eq!(row.1, "unassigned");
        assert_eq!(row.2.as_deref(), Some("ops"));
        assert_eq!(row.3.as_deref(), Some("run_name"));
        let evidence: Vec<Evidence> = serde_json::from_str(&row.4).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].score, 100);
    }

    #[test]
    fn historical_reconciliation_does_not_log_unchanged_decision() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               track_key TEXT NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE resolution_reconciliation_audit(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
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
             INSERT INTO workflow_runs VALUES(501,100,'unassigned',NULL);"
        ).unwrap();

        let resolution = Resolution {
            status: "unassigned".into(),
            track_id: None,
            confidence: Some(40),
            source: None,
            reason: Some("확정 가능한 Track Key 근거가 없습니다.".into()),
            evidence: vec![evidence("ops", "workflow_name", 40)],
        };
        persist_resolution_with_trigger(
            &conn,
            501,
            &resolution,
            "2026-09-24T02:00:00Z",
            Some("historical_reconcile"),
        ).unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM resolution_reconciliation_audit WHERE run_id=501",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn historical_reconciliation_rotates_unresolved_runs_without_manual_starvation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               workflow_id INTEGER NOT NULL,
               workflow_name TEXT NOT NULL,
               workflow_path TEXT,
               display_title TEXT,
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
               ignored INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               manual INTEGER NOT NULL
             );"
        ).unwrap();

        for id in 1_i64..=15 {
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,
                   display_title,event,head_branch,head_sha,run_number,run_attempt,
                   status,conclusion,html_url,created_at,run_started_at,updated_at,
                   ignored,resolution_status,last_resolution_attempt_at
                 ) VALUES(?,100,1,'CI',NULL,'CI','push',NULL,?, ?,1,
                          'completed','success',?, ?,NULL,?,0,'unassigned',NULL)",
                params![
                    id,
                    format!("sha-{id}"),
                    id,
                    format!("https://example.invalid/{id}"),
                    format!("2026-09-{:02}T00:00:00Z", id),
                    format!("2026-09-{:02}T00:05:00Z", id)
                ],
            ).unwrap();
        }

        conn.execute_batch(
            "INSERT INTO workflow_runs(
               run_id,repository_id,workflow_id,workflow_name,workflow_path,
               display_title,event,head_branch,head_sha,run_number,run_attempt,
               status,conclusion,html_url,created_at,run_started_at,updated_at,
               ignored,resolution_status,last_resolution_attempt_at
             ) VALUES(99,100,1,'CI',NULL,'CI','push',NULL,'sha-99',99,1,
                      'completed','success','https://example.invalid/99',
                      '2026-09-30T00:00:00Z',NULL,'2026-09-30T00:05:00Z',
                      0,'unassigned',NULL);
             INSERT INTO run_assignments(run_id,manual) VALUES(99,1);"
        ).unwrap();

        let first = load_stored_unresolved_runs(&conn, 100, 12).unwrap();
        assert_eq!(first.len(), 12);
        assert!(!first.iter().any(|run| run.id == 99));

        for run in &first {
            conn.execute(
                "UPDATE workflow_runs
                 SET last_resolution_attempt_at='2026-09-24T01:00:00Z'
                 WHERE run_id=?",
                params![run.id],
            ).unwrap();
        }

        let second = load_stored_unresolved_runs(&conn, 100, 12).unwrap();
        let first_three: Vec<i64> = second.iter().take(3).map(|run| run.id).collect();
        assert_eq!(first_three, vec![3, 2, 1]);
        assert!(!second.iter().any(|run| run.id == 99));
    }

    #[test]
    fn project_move_archives_manual_assignment_before_invalidating_it() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER,
               repo TEXT NOT NULL
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               name TEXT NOT NULL,
               track_key TEXT NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               ignored INTEGER NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE assignment_migration_audit(
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
               UNIQUE(migration_key,run_id,track_id)
             );
             INSERT INTO monitored_repositories(id,project_id,repo)
               VALUES(100,1,'gycha0109-beep/K_beauty');
             INSERT INTO watch_tracks(id,project_id,name,track_key)
               VALUES(10,1,'Old Manual Track','old-track'),
                     (20,2,'Visualy Ops','ops');
             INSERT INTO workflow_runs(run_id,repository_id,resolution_status,ignored)
               VALUES(500,100,'assigned',0);
             INSERT INTO run_assignments(
               run_id,track_id,confidence,source,reason,manual,assigned_at
             ) VALUES(500,10,100,'manual','사용자 수동 귀속',1,'2026-09-23T00:00:00Z');"
        ).unwrap();

        invalidate_cross_project_assignments(
            &conn,
            100,
            Some(1),
            2,
            "visualy-project-scope-v1",
        ).unwrap();

        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(remaining, 0);

        let archived: (i64, String, String, i64, i64) = conn.query_row(
            "SELECT manual,track_key,reason,from_repository_project_id,to_repository_project_id
             FROM assignment_migration_audit
             WHERE migration_key='visualy-project-scope-v1' AND run_id=500",
            [],
            |row| Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            )),
        ).unwrap();
        assert_eq!(archived.0, 1);
        assert_eq!(archived.1, "old-track");
        assert_eq!(archived.2, "사용자 수동 귀속");
        assert_eq!(archived.3, 1);
        assert_eq!(archived.4, 2);

        let status: String = conn.query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=500",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "unassigned");
    }

    #[test]
    fn project_move_audit_is_idempotent_and_keeps_same_project_manual_assignment() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER,
               repo TEXT NOT NULL
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               name TEXT NOT NULL,
               track_key TEXT NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               resolution_status TEXT NOT NULL,
               ignored INTEGER NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE assignment_migration_audit(
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
               UNIQUE(migration_key,run_id,track_id)
             );
             INSERT INTO monitored_repositories(id,project_id,repo)
               VALUES(100,1,'gycha0109-beep/K_beauty');
             INSERT INTO watch_tracks(id,project_id,name,track_key)
               VALUES(10,1,'Old Auto','old-auto'),
                     (20,2,'Visualy Manual','ops');
             INSERT INTO workflow_runs(run_id,repository_id,resolution_status,ignored)
               VALUES(500,100,'assigned',0),
                     (600,100,'assigned',0);
             INSERT INTO run_assignments(
               run_id,track_id,confidence,source,reason,manual,assigned_at
             ) VALUES(500,10,90,'branch','old automatic',0,'2026-09-23T00:00:00Z'),
                     (600,20,100,'manual','valid visualy manual',1,'2026-09-23T00:00:00Z');"
        ).unwrap();

        invalidate_cross_project_assignments(
            &conn,
            100,
            Some(1),
            2,
            "visualy-project-scope-v1",
        ).unwrap();
        invalidate_cross_project_assignments(
            &conn,
            100,
            Some(1),
            2,
            "visualy-project-scope-v1",
        ).unwrap();

        let audit_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM assignment_migration_audit",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(audit_count, 1);

        let valid_manual_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM run_assignments
             WHERE run_id=600 AND track_id=20 AND manual=1",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(valid_manual_count, 1);

        let invalid_auto_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(invalid_auto_count, 0);
    }

    #[test]
    fn myeongha_repository_scoped_project_wide_rules_preserve_manual_and_repo_isolation() {
        let path = legacy_v02_db_path("myeongha-repository-project-wide");
        seed_legacy_v02_database(&path);
        init_db(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let myeongha_repository_id: i64 = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/MyeongHa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let saju_repository_id: i64 = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/Saju'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM monitored_repositories WHERE id=?",
                params![myeongha_repository_id],
                |row| row.get(0),
            )
            .unwrap();
        let ops_track_id: i64 = conn
            .query_row(
                "SELECT id FROM watch_tracks WHERE project_id=? AND track_key='ops'",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap();

        for (run_id, repository_id, manual) in [
            (9_100_001_i64, myeongha_repository_id, 0_i64),
            (9_100_002_i64, myeongha_repository_id, 1_i64),
            (9_100_003_i64, saju_repository_id, 0_i64),
        ] {
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
                   head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
                   created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    run_id,
                    repository_id,
                    910_i64,
                    "Supabase Production",
                    ".github/workflows/supabase-production.yml",
                    "supabase production",
                    "push",
                    "main",
                    format!("sha-{run_id}"),
                    run_id,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    "2026-09-24T01:00:00Z",
                    Option::<String>::None,
                    "2026-09-24T01:01:00Z",
                    "2026-09-24T01:01:00Z",
                    "assigned",
                    0_i64,
                    Option::<String>::None,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO run_assignments(
                   run_id,track_id,confidence,source,reason,manual,assigned_at
                 ) VALUES(?,?,?,?,?,?,?)",
                params![
                    run_id,
                    ops_track_id,
                    if manual == 1 { 100_i64 } else { 90_i64 },
                    if manual == 1 { "manual" } else { "branch" },
                    "fixture",
                    manual,
                    "2026-09-24T01:02:00Z",
                ],
            )
            .unwrap();
        }

        for (run_id, repository_id) in [
            (9_100_004_i64, myeongha_repository_id),
            (9_100_005_i64, saju_repository_id),
        ] {
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
                   head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
                   created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    run_id,
                    repository_id,
                    911_i64,
                    "Web Browser Smoke",
                    ".github/workflows/web-browser-render-smoke.yml",
                    "web browser smoke",
                    "push",
                    "main",
                    format!("sha-{run_id}"),
                    run_id,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    "2026-09-24T01:00:00Z",
                    Option::<String>::None,
                    "2026-09-24T01:01:00Z",
                    "2026-09-24T01:01:00Z",
                    "assigned",
                    0_i64,
                    Option::<String>::None,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO run_assignments(
                   run_id,track_id,confidence,source,reason,manual,assigned_at
                 ) VALUES(?,?,?,?,?,0,?)",
                params![
                    run_id,
                    ops_track_id,
                    90_i64,
                    "branch",
                    "fixture",
                    "2026-09-24T01:02:00Z",
                ],
            )
            .unwrap();
        }
        drop(conn);

        migrate_project_scope(&Connection::open(&path).unwrap()).unwrap();

        let conn = Connection::open(&path).unwrap();
        let scoped_rules: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_workflow_rules
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name IN (
                     'DB Content Reading Suite',
                     'DB Runtime Authority Suite',
                     'DB PostgreSQL 17 Authority Suite',
                     'Supabase Production',
                     'Web Browser Smoke'
                   )
                   AND active=1",
                params![project_id, myeongha_repository_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scoped_rules, 5);

        let automatic_status: String = conn
            .query_row(
                "SELECT resolution_status FROM workflow_runs WHERE run_id=9100001",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let automatic_assignment_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_assignments WHERE run_id=9100001",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(automatic_status, "project");
        assert_eq!(automatic_assignment_count, 0);

        let manual_row: (String, i64) = conn
            .query_row(
                "SELECT wr.resolution_status,ra.manual
                 FROM workflow_runs wr
                 JOIN run_assignments ra ON ra.run_id=wr.run_id
                 WHERE wr.run_id=9100002",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(manual_row, ("assigned".into(), 1));

        let saju_row: (String, i64) = conn
            .query_row(
                "SELECT wr.resolution_status,COUNT(ra.run_id)
                 FROM workflow_runs wr
                 LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
                 WHERE wr.run_id=9100003
                 GROUP BY wr.run_id,wr.resolution_status",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(saju_row, ("assigned".into(), 1));

        let web_browser_status: String = conn
            .query_row(
                "SELECT resolution_status FROM workflow_runs WHERE run_id=9100004",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let web_browser_assignment_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_assignments WHERE run_id=9100004",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(web_browser_status, "project");
        assert_eq!(web_browser_assignment_count, 0);

        let saju_web_browser_row: (String, i64) = conn
            .query_row(
                "SELECT wr.resolution_status,COUNT(ra.run_id)
                 FROM workflow_runs wr
                 LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
                 WHERE wr.run_id=9100005
                 GROUP BY wr.run_id,wr.resolution_status",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(saju_web_browser_row, ("assigned".into(), 1));

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn visualy_security_boundary_is_project_wide_and_preserves_manual_override() {
        let path = legacy_v02_db_path("visualy-security-boundary");
        init_db(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let repository_id: i64 = conn
            .query_row(
                "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/K_beauty'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM monitored_repositories WHERE id=?",
                params![repository_id],
                |row| row.get(0),
            )
            .unwrap();
        let ops_track_id: i64 = conn
            .query_row(
                "SELECT id FROM watch_tracks WHERE project_id=? AND track_key='ops'",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap();

        for (run_id, manual) in [(9_200_001_i64, 0_i64), (9_200_002_i64, 1_i64)] {
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
                   head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
                   created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    run_id,
                    repository_id,
                    920_i64,
                    "BEJEWELY Security Boundary",
                    ".github/workflows/security-boundary.yml",
                    "security boundary",
                    "pull_request",
                    "ci/security-boundary-ownership",
                    format!("sha-{run_id}"),
                    run_id,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    "2026-09-24T01:00:00Z",
                    Option::<String>::None,
                    "2026-09-24T01:01:00Z",
                    "2026-09-24T01:01:00Z",
                    "assigned",
                    0_i64,
                    Option::<String>::None,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO run_assignments(
                   run_id,track_id,confidence,source,reason,manual,assigned_at
                 ) VALUES(?,?,?,?,?,?,?)",
                params![
                    run_id,
                    ops_track_id,
                    if manual == 1 { 100_i64 } else { 90_i64 },
                    if manual == 1 { "manual" } else { "branch" },
                    "fixture",
                    manual,
                    "2026-09-24T01:02:00Z",
                ],
            )
            .unwrap();
        }
        drop(conn);

        seed_bejewely_project_scope(&Connection::open(&path).unwrap()).unwrap();

        let conn = Connection::open(&path).unwrap();
        let rule_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_workflow_rules
                 WHERE project_id=? AND repository_id IS NULL
                   AND workflow_name='BEJEWELY Security Boundary' AND active=1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rule_count, 1);

        let automatic_status: String = conn
            .query_row(
                "SELECT resolution_status FROM workflow_runs WHERE run_id=9200001",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let automatic_assignment_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_assignments WHERE run_id=9200001",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(automatic_status, "project");
        assert_eq!(automatic_assignment_count, 0);

        let manual_row: (String, i64) = conn
            .query_row(
                "SELECT wr.resolution_status,ra.manual
                 FROM workflow_runs wr
                 JOIN run_assignments ra ON ra.run_id=wr.run_id
                 WHERE wr.run_id=9200002",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(manual_row, ("assigned".into(), 1));

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn producer_contract_stats_classify_recent_runs_without_overlapping_buckets() {
        let path = legacy_v02_db_path("producer-contract");
        init_db(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute("PRAGMA foreign_keys=ON", []).unwrap();
        let repository_id: i64 = conn
            .query_row(
                "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/K_beauty'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM monitored_repositories WHERE id=?",
                params![repository_id],
                |row| row.get(0),
            )
            .unwrap();
        let track_id: i64 = conn
            .query_row(
                "SELECT id FROM watch_tracks
                 WHERE project_id=? AND track_key='ops'",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap();

        for (offset, status) in [
            (1_i64, "project"),
            (2, "assigned"),
            (3, "assigned"),
            (4, "assigned"),
            (5, "assigned"),
            (6, "assigned"),
            (7, "assigned"),
            (8, "unassigned"),
            (9, "assigned"),
        ] {
            let run_id = 9_100_000 + offset;
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
                   head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
                   created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    run_id,
                    repository_id,
                    700 + offset,
                    format!("Contract {offset}"),
                    Option::<String>::None,
                    format!("Contract {offset}"),
                    "push",
                    "main",
                    format!("sha-{offset}"),
                    offset,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    format!("2026-09-24T00:{offset:02}:00Z"),
                    Option::<String>::None,
                    format!("2026-09-24T00:{offset:02}:30Z"),
                    format!("2026-09-24T00:{offset:02}:30Z"),
                    status,
                    if offset == 9 { 1_i64 } else { 0_i64 },
                    Option::<String>::None,
                ],
            )
            .unwrap();
        }

        for (offset, source, manual) in [
            (2_i64, "run_name", 0_i64),
            (3, "pr_marker", 0),
            (4, "branch", 0),
            (5, "inference", 0),
            (6, "manual", 1),
            (7, "track_alias", 0),
            (9, "commit_marker", 0),
        ] {
            conn.execute(
                "INSERT INTO run_assignments(
                   run_id,track_id,confidence,source,reason,manual,assigned_at
                 ) VALUES(?,?,?,?,?,?,?)",
                params![
                    9_100_000 + offset,
                    track_id,
                    100_i64,
                    source,
                    "fixture",
                    manual,
                    "2026-09-24T00:10:00Z",
                ],
            )
            .unwrap();
        }

        let stats = producer_contract_stats(&conn, 50).unwrap();
        let row = stats
            .iter()
            .find(|item| item.repository_id == repository_id)
            .unwrap();
        assert_eq!(row.sampled_runs, 8);
        assert_eq!(row.project_wide_runs, 1);
        assert_eq!(row.explicit_runs, 3);
        assert_eq!(row.run_name_runs, 1);
        assert_eq!(row.pr_marker_runs, 1);
        assert_eq!(row.commit_marker_runs, 0);
        assert_eq!(row.branch_runs, 1);
        assert_eq!(row.heuristic_runs, 1);
        assert_eq!(row.manual_runs, 1);
        assert_eq!(row.compatibility_runs, 1);
        assert_eq!(row.unresolved_runs, 1);
        assert_eq!(row.other_runs, 0);
        assert_eq!(
            row.project_wide_runs
                + row.explicit_runs
                + row.heuristic_runs
                + row.manual_runs
                + row.compatibility_runs
                + row.unresolved_runs
                + row.other_runs,
            row.sampled_runs
        );

        let contract_runs = producer_contract_runs(&conn, 50).unwrap();
        let repository_runs: Vec<&ProducerContractRun> = contract_runs
            .iter()
            .filter(|item| item.run.repository_id == repository_id)
            .collect();
        assert_eq!(repository_runs.len(), 8);
        assert_eq!(
            repository_runs.iter().filter(|item| item.contract_compliant).count(),
            4
        );
        let drift_buckets: Vec<&str> = repository_runs
            .iter()
            .filter(|item| !item.contract_compliant)
            .map(|item| item.bucket.as_str())
            .collect();
        assert!(drift_buckets.contains(&"inference"));
        assert!(drift_buckets.contains(&"manual"));
        assert!(drift_buckets.contains(&"track_alias"));
        assert!(drift_buckets.contains(&"unassigned"));
        assert!(!repository_runs.iter().any(|item| item.run.id == 9_100_009));

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn legacy_v02_database_migrates_end_to_end_without_silent_data_loss() {
        let path = legacy_v02_db_path("v02-e2e");
        seed_legacy_v02_database(&path);

        init_db(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute("PRAGMA foreign_keys=ON", []).unwrap();

        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");

        let foreign_key_violations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(foreign_key_violations, 0);

        let run_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 5);

        let myeongha_project_id: i64 = conn
            .query_row(
                "SELECT id FROM projects WHERE project_key='myeongha'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let visualy_project_id: i64 = conn
            .query_row(
                "SELECT id FROM projects WHERE project_key='visualy'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let myeongha_repo_project: i64 = conn
            .query_row(
                "SELECT project_id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/MyeongHa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let visualy_repo_project: i64 = conn
            .query_row(
                "SELECT project_id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/K_beauty'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(myeongha_repo_project, myeongha_project_id);
        assert_eq!(visualy_repo_project, visualy_project_id);

        let commerce_track: (String, i64) = conn
            .query_row(
                "SELECT track_key,project_id FROM watch_tracks
                 WHERE name='결제'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(commerce_track.0, "product-commerce");
        assert_eq!(commerce_track.1, myeongha_project_id);

        let alias_targets: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT ta.alias_key,wt.track_key
                     FROM track_aliases ta
                     JOIN watch_tracks wt ON wt.id=ta.track_id
                     WHERE ta.project_id=?
                     ORDER BY ta.alias_key",
                )
                .unwrap();
            let rows = stmt
                .query_map(params![myeongha_project_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(alias_targets.contains(&("privacy-recovery".into(), "ops".into())));
        assert!(alias_targets.contains(&("commerce".into(), "product-commerce".into())));

        let privacy_assignment: (String, i64, String) = conn
            .query_row(
                "SELECT wt.track_key,ra.manual,ra.source
                 FROM run_assignments ra
                 JOIN watch_tracks wt ON wt.id=ra.track_id
                 WHERE ra.run_id=501",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(privacy_assignment, ("ops".into(), 0, "track_alias".into()));

        let commerce_assignment: (String, i64, String) = conn
            .query_row(
                "SELECT wt.track_key,ra.manual,ra.source
                 FROM run_assignments ra
                 JOIN watch_tracks wt ON wt.id=ra.track_id
                 WHERE ra.run_id=502",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            commerce_assignment,
            ("product-commerce".into(), 0, "track_alias".into())
        );

        let invalid_manual_active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_assignments WHERE run_id=500",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invalid_manual_active, 0);

        let manual_audit: (String, i64, String, i64, i64) = conn
            .query_row(
                "SELECT track_key,manual,reason,from_repository_project_id,to_repository_project_id
                 FROM assignment_migration_audit
                 WHERE migration_key='visualy-project-scope-v1' AND run_id=500",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(manual_audit.0, "ops");
        assert_eq!(manual_audit.1, 1);
        assert_eq!(manual_audit.2, "legacy user choice");
        assert_eq!(manual_audit.3, myeongha_project_id);
        assert_eq!(manual_audit.4, visualy_project_id);

        let moved_manual_status: String = conn
            .query_row(
                "SELECT resolution_status FROM workflow_runs WHERE run_id=500",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(moved_manual_status, "unassigned");

        for run_id in [503_i64, 504_i64] {
            let status: String = conn
                .query_row(
                    "SELECT resolution_status FROM workflow_runs WHERE run_id=?",
                    params![run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, "project");
            let assignment_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM run_assignments WHERE run_id=?",
                    params![run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(assignment_count, 0);
        }

        let fingerprint_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM track_fingerprints", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fingerprint_count, 2);

        let notification_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notifications_v2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(notification_count, 1);

        drop(conn);

        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();

        let run_count_after_second_init: i64 = conn
            .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
            .unwrap();
        let migration_audit_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM assignment_migration_audit
                 WHERE migration_key='visualy-project-scope-v1' AND run_id=500",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let alias_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_aliases
                 WHERE project_id=? AND alias_key IN ('privacy-recovery','commerce')",
                params![myeongha_project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(run_count_after_second_init, 5);
        assert_eq!(migration_audit_count, 1);
        assert_eq!(alias_count, 2);

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn alias_migration_is_project_scoped_and_preserves_manual_assignment() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects(
               id INTEGER PRIMARY KEY,
               project_key TEXT NOT NULL UNIQUE
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               name TEXT NOT NULL,
               track_key TEXT NOT NULL,
               long_ci_minutes INTEGER NOT NULL,
               active INTEGER NOT NULL,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE track_aliases(
               alias_key TEXT PRIMARY KEY,
               track_id INTEGER NOT NULL,
               active INTEGER NOT NULL DEFAULT 1,
               created_at TEXT NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               manual INTEGER NOT NULL
             );
             INSERT INTO projects(id,project_key) VALUES(1,'one'),(2,'two');
             INSERT INTO watch_tracks(id,project_id,name,track_key,long_ci_minutes,active,created_at,updated_at)
               VALUES(10,1,'Ops A','ops',8,1,'now','now'),
                     (20,2,'Ops B','ops',8,1,'now','now');
             INSERT INTO track_aliases(alias_key,track_id,active,created_at)
               VALUES('privacy-recovery',10,1,'now');
             INSERT INTO run_assignments(run_id,track_id,manual) VALUES(500,10,1);"
        ).unwrap();

        migrate_track_alias_scope(&conn).unwrap();

        let migrated_project_id: i64 = conn.query_row(
            "SELECT project_id FROM track_aliases WHERE alias_key='privacy-recovery'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(migrated_project_id, 1);

        conn.execute(
            "INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at)
             VALUES(2,'privacy-recovery',20,1,'now')",
            [],
        ).unwrap();

        let one = load_project_aliases(&conn, 1).unwrap();
        let two = load_project_aliases(&conn, 2).unwrap();
        assert_eq!(one.get("privacy-recovery").map(String::as_str), Some("ops"));
        assert_eq!(two.get("privacy-recovery").map(String::as_str), Some("ops"));

        let mismatch = conn.execute(
            "INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at)
             VALUES(1,'wrong-project',20,1,'now')",
            [],
        );
        assert!(mismatch.is_err());

        let manual: i64 = conn.query_row(
            "SELECT manual FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(manual, 1);
    }

    #[test]
    fn fingerprints_are_isolated_by_project_and_repository() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               active INTEGER NOT NULL
             );
             CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL
             );
             CREATE TABLE track_fingerprints(
               id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               signal_type TEXT NOT NULL,
               pattern TEXT NOT NULL,
               repository_id INTEGER,
               weight INTEGER NOT NULL,
               active INTEGER NOT NULL
             );
             INSERT INTO watch_tracks(id,project_id,track_key,active)
               VALUES(10,1,'project-one',1),(20,2,'project-two',1);
             INSERT INTO monitored_repositories(id,project_id)
               VALUES(101,1),(202,2);
             INSERT INTO track_fingerprints(id,track_id,signal_type,pattern,repository_id,weight,active)
               VALUES(1,10,'workflow_name','global-one',NULL,50,1),
                     (2,20,'workflow_name','global-two',NULL,50,1),
                     (3,10,'workflow_name','repo-one',101,50,1),
                     (4,20,'workflow_name','cross-track',101,50,1),
                     (5,10,'workflow_name','wrong-repo',202,50,1);"
        ).unwrap();

        let project_one = load_fingerprints(&conn, 1, 101).unwrap();
        let patterns: HashSet<_> = project_one.iter().map(|fp| fp.pattern.as_str()).collect();
        assert_eq!(patterns.len(), 2);
        assert!(patterns.contains("global-one"));
        assert!(patterns.contains("repo-one"));
        assert!(!patterns.contains("global-two"));
        assert!(!patterns.contains("cross-track"));
        assert!(!patterns.contains("wrong-repo"));

        let project_two = load_fingerprints(&conn, 2, 202).unwrap();
        let patterns: HashSet<_> = project_two.iter().map(|fp| fp.pattern.as_str()).collect();
        assert_eq!(patterns, HashSet::from(["global-two"]));

        assert!(load_fingerprints(&conn, 1, 202).is_err());
    }

    #[test]
    fn attribution_detail_exposes_final_decision_and_ordered_evidence() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               repo TEXT NOT NULL
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               name TEXT NOT NULL,
               track_key TEXT NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               workflow_name TEXT NOT NULL,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE project_workflow_rules(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               repository_id INTEGER,
               workflow_name TEXT NOT NULL,
               active INTEGER NOT NULL
             );
             INSERT INTO monitored_repositories VALUES(100,1,'example/repo');
             INSERT INTO watch_tracks VALUES(10,1,'운영','ops');
             INSERT INTO workflow_runs VALUES(1000,100,'CI','assigned','2026-09-24T02:00:00Z');
             INSERT INTO run_assignments VALUES(1000,10,100,'manual','사용자 수동 귀속',1,'2026-09-24T02:01:00Z');
             INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) VALUES
               (1000,'ops','branch',90,'feat/ops/a','2026-09-24T02:00:00Z'),
               (1000,'ops','run_name',100,'[WT:ops] CI','2026-09-24T02:00:00Z');"
        ).unwrap();

        let detail = load_run_attribution_detail(&conn, 1000).unwrap();
        assert_eq!(detail.project_id, 1);
        assert_eq!(detail.repository, "example/repo");
        assert_eq!(detail.assigned_track_key.as_deref(), Some("ops"));
        assert_eq!(detail.source.as_deref(), Some("manual"));
        assert_eq!(detail.confidence, Some(100));
        assert_eq!(detail.manual, Some(true));
        assert_eq!(detail.evidence.len(), 2);
        assert_eq!(detail.evidence[0].signal_type, "run_name");
        assert_eq!(detail.evidence[0].score, 100);
        assert_eq!(detail.evidence[1].signal_type, "branch");
    }

    #[test]
    fn attribution_detail_identifies_repository_specific_project_rule() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               repo TEXT NOT NULL
             );
             CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               name TEXT NOT NULL,
               track_key TEXT NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               workflow_name TEXT NOT NULL,
               resolution_status TEXT NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE project_workflow_rules(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               repository_id INTEGER,
               workflow_name TEXT NOT NULL,
               active INTEGER NOT NULL
             );
             INSERT INTO monitored_repositories VALUES(100,1,'example/repo');
             INSERT INTO workflow_runs VALUES(1000,100,'Governance','project','2026-09-24T02:00:00Z');
             INSERT INTO project_workflow_rules VALUES(7,1,NULL,'Governance',1);
             INSERT INTO project_workflow_rules VALUES(8,1,100,'Governance',1);"
        ).unwrap();

        let detail = load_run_attribution_detail(&conn, 1000).unwrap();
        assert_eq!(detail.resolution_status, "project");
        assert_eq!(detail.source.as_deref(), Some("project_workflow"));
        assert_eq!(detail.confidence, Some(100));
        assert_eq!(detail.project_rule_id, Some(8));
        assert_eq!(detail.project_rule_repository_id, Some(100));
        assert!(detail.evidence.is_empty());
    }

    #[test]
    fn workflow_name_fingerprint_alone_stays_below_auto_assignment_threshold() {
        let tracks = vec![track(10, "ops")];
        let fingerprints = vec![Fingerprint {
            track_key: "ops".into(),
            signal_type: "workflow_name".into(),
            pattern: "Shared CI".into(),
            weight: 50,
        }];
        let run = GithubRun {
            id: 101,
            workflow_id: 1,
            name: "Shared CI".into(),
            path: None,
            display_title: Some("Shared CI".into()),
            event: "push".into(),
            head_branch: Some("main".into()),
            head_sha: "sha-101".into(),
            run_number: 101,
            run_attempt: 1,
            status: "completed".into(),
            conclusion: Some("success".into()),
            html_url: "https://github.com/example/repo/actions/runs/101".into(),
            created_at: "2026-09-24T01:00:00Z".into(),
            run_started_at: Some("2026-09-24T01:00:01Z".into()),
            updated_at: "2026-09-24T01:01:00Z".into(),
            pull_requests: Vec::new(),
        };

        let resolution = resolve_evidence(
            &tracks,
            &HashMap::new(),
            fingerprint_evidence(&fingerprints, &run),
        );

        assert_eq!(resolution.status, "unassigned");
        assert_eq!(resolution.track_id, None);
        assert_eq!(resolution.confidence, Some(50));
    }

    #[test]
    fn manual_assignment_learning_and_project_wide_transition_complete_acceptance_cycle() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE watch_tracks(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               active INTEGER NOT NULL
             );
             CREATE TABLE monitored_repositories(
               id INTEGER PRIMARY KEY,
               project_id INTEGER NOT NULL
             );
             CREATE TABLE workflow_runs(
               run_id INTEGER PRIMARY KEY,
               repository_id INTEGER NOT NULL,
               workflow_name TEXT NOT NULL,
               workflow_path TEXT,
               resolution_status TEXT NOT NULL,
               ignored INTEGER NOT NULL,
               last_resolution_attempt_at TEXT
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL,
               confidence INTEGER NOT NULL,
               source TEXT NOT NULL,
               reason TEXT NOT NULL,
               manual INTEGER NOT NULL,
               assigned_at TEXT NOT NULL
             );
             CREATE TABLE run_evidence(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               run_id INTEGER NOT NULL,
               track_key TEXT NOT NULL,
               signal_type TEXT NOT NULL,
               score INTEGER NOT NULL,
               value TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE track_fingerprints(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               track_id INTEGER NOT NULL,
               signal_type TEXT NOT NULL,
               pattern TEXT NOT NULL,
               repository_id INTEGER,
               weight INTEGER NOT NULL,
               learned_from_run_id INTEGER,
               active INTEGER NOT NULL,
               created_at TEXT NOT NULL,
               UNIQUE(track_id,signal_type,pattern,repository_id)
             );
             CREATE TABLE project_workflow_rules(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL,
               repository_id INTEGER,
               workflow_name TEXT NOT NULL,
               active INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE UNIQUE INDEX idx_test_project_workflow_rules_scope
               ON project_workflow_rules(project_id,COALESCE(repository_id,0),workflow_name);

             INSERT INTO watch_tracks(id,project_id,track_key,active)
               VALUES(10,1,'ops',1),(20,2,'ops',1);
             INSERT INTO monitored_repositories(id,project_id)
               VALUES(100,1),(110,1),(200,2);
             INSERT INTO workflow_runs(
               run_id,repository_id,workflow_name,workflow_path,resolution_status,ignored,last_resolution_attempt_at
             ) VALUES
               (1000,100,'Shared CI','.github/workflows/shared.yml','unassigned',0,NULL),
               (1001,100,'Shared CI','.github/workflows/shared.yml','unassigned',0,NULL),
               (2000,200,'Shared CI','.github/workflows/shared.yml','unassigned',0,NULL);"
        ).unwrap();

        assign_run_in_conn(&conn, 1000, 10, "2026-09-24T01:00:00Z").unwrap();

        let manual: (i64, i64, String) = conn.query_row(
            "SELECT track_id,manual,source FROM run_assignments WHERE run_id=1000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(manual, (10, 1, "manual".into()));

        let learned = load_fingerprints(&conn, 1, 100).unwrap();
        let learned_signals: HashMap<_, _> = learned
            .iter()
            .map(|item| (item.signal_type.as_str(), item.weight))
            .collect();
        assert_eq!(learned_signals.get("workflow_name"), Some(&50));
        assert_eq!(learned_signals.get("workflow_path"), Some(&35));
        assert!(load_fingerprints(&conn, 2, 200).unwrap().is_empty());

        let next_run = GithubRun {
            id: 1001,
            workflow_id: 1,
            name: "Shared CI".into(),
            path: Some(".github/workflows/shared.yml".into()),
            display_title: Some("Shared CI".into()),
            event: "push".into(),
            head_branch: Some("main".into()),
            head_sha: "sha-1001".into(),
            run_number: 2,
            run_attempt: 1,
            status: "completed".into(),
            conclusion: Some("success".into()),
            html_url: "https://github.com/example/repo/actions/runs/1001".into(),
            created_at: "2026-09-24T01:10:00Z".into(),
            run_started_at: Some("2026-09-24T01:10:01Z".into()),
            updated_at: "2026-09-24T01:11:00Z".into(),
            pull_requests: Vec::new(),
        };
        let inferred = resolve_evidence(
            &[track(10, "ops")],
            &HashMap::new(),
            fingerprint_evidence(&learned, &next_run),
        );
        assert_eq!(inferred.status, "assigned");
        assert_eq!(inferred.track_id, Some(10));
        assert_eq!(inferred.confidence, Some(85));
        assert_eq!(inferred.source.as_deref(), Some("inference"));

        persist_resolution(&conn, 1001, &inferred, "2026-09-24T01:11:00Z").unwrap();
        let automatic: (i64, i64, String) = conn.query_row(
            "SELECT track_id,manual,source FROM run_assignments WHERE run_id=1001",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(automatic, (10, 0, "inference".into()));

        assign_run_to_project_in_conn(
            &conn,
            1001,
            1,
            true,
            "2026-09-24T01:12:00Z",
        ).unwrap();

        let rules = list_project_workflow_rules(&conn).unwrap();
        assert!(project_rule_matches(&rules, 1, 100, "Shared CI"));
        assert!(project_rule_matches(&rules, 1, 110, "Shared CI"));
        assert!(!project_rule_matches(&rules, 2, 200, "Shared CI"));

        let promoted_status: String = conn.query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=1001",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(promoted_status, "project");
        let promoted_assignment_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=1001",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(promoted_assignment_count, 0);

        let preserved_manual: (String, i64) = conn.query_row(
            "SELECT wr.resolution_status,ra.manual
             FROM workflow_runs wr
             JOIN run_assignments ra ON ra.run_id=wr.run_id
             WHERE wr.run_id=1000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(preserved_manual, ("assigned".into(), 1));

        let other_project_status: String = conn.query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=2000",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(other_project_status, "unassigned");
    }

    #[test]
    fn explicit_conflict_model_has_distinct_keys() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let known: HashSet<_> = tracks.iter().map(|t| t.track_key.as_str()).collect();
        assert!(known.contains("ops"));
        assert!(known.contains("saju"));
    }
}
