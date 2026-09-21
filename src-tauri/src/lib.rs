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

struct AppState {
    db_path: PathBuf,
    poll_in_flight: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    id: i64,
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
    name: String,
    track_key: String,
    long_ci_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitoredRepository {
    id: i64,
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
    repo: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunSummary {
    id: i64,
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
    repositories: Vec<MonitoredRepository>,
    tracks: Vec<DashboardTrack>,
    unassigned_runs: Vec<WorkflowRunSummary>,
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

#[derive(Debug, Clone)]
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

        CREATE TABLE IF NOT EXISTS watch_tracks (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    conn.execute(
        "INSERT OR IGNORE INTO app_settings(id, queue_congestion_threshold, active_poll_seconds, idle_poll_seconds, auto_archive_completed, queue_congested) VALUES(1,?,?,?,?,0)",
        params![DEFAULT_QUEUE_THRESHOLD, DEFAULT_ACTIVE_POLL_SECONDS, DEFAULT_IDLE_POLL_SECONDS, 0],
    )?;
    migrate_legacy(&conn)?;
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
        "commerce".into()
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
        .user_agent("ci-watchtower/0.2.0")
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

fn extract_marker(text: &str) -> Option<String> {
    let start = text.find("[WT:")? + 4;
    let tail = &text[start..];
    let end = tail.find(']')?;
    let key = tail[..end].trim().to_lowercase();
    validate_track_key(&key).ok()?;
    Some(key)
}

fn extract_track_trailer(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Watchtower-Track:") {
            let key = value.trim().to_lowercase();
            if validate_track_key(&key).is_ok() {
                return Some(key);
            }
        }
    }
    None
}

fn branch_has_key(branch: &str, key: &str) -> bool {
    branch == key
        || branch.split('/').any(|segment| segment == key)
        || branch.starts_with(&format!("{key}/"))
        || branch.ends_with(&format!("/{key}"))
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

fn list_tracks(conn: &Connection, active_only: bool) -> Result<Vec<Track>> {
    let sql = if active_only {
        "SELECT id,name,track_key,long_ci_minutes,active,created_at,updated_at FROM watch_tracks WHERE active=1 ORDER BY id DESC"
    } else {
        "SELECT id,name,track_key,long_ci_minutes,active,created_at,updated_at FROM watch_tracks ORDER BY id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Track {
            id: row.get(0)?,
            name: row.get(1)?,
            track_key: row.get(2)?,
            long_ci_minutes: row.get(3)?,
            active: row.get::<_, i64>(4)? != 0,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_repositories(conn: &Connection, enabled_only: bool) -> Result<Vec<MonitoredRepository>> {
    let sql = if enabled_only {
        "SELECT id,repo,enabled,running_count,queued_count,last_polled_at,last_error FROM monitored_repositories WHERE enabled=1 ORDER BY repo"
    } else {
        "SELECT id,repo,enabled,running_count,queued_count,last_polled_at,last_error FROM monitored_repositories ORDER BY repo"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(MonitoredRepository {
            id: row.get(0)?,
            repo: row.get(1)?,
            enabled: row.get::<_, i64>(2)? != 0,
            running_count: row.get(3)?,
            queued_count: row.get(4)?,
            last_polled_at: row.get(5)?,
            last_error: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn run_summary_from_row(row: &rusqlite::Row<'_>, now: DateTime<Utc>) -> rusqlite::Result<WorkflowRunSummary> {
    let status: String = row.get(8)?;
    let created_at: String = row.get(12)?;
    let run_started_at: Option<String> = row.get(13)?;
    let updated_at: String = row.get(14)?;
    Ok(WorkflowRunSummary {
        id: row.get(0)?,
        repository: row.get(1)?,
        workflow_name: row.get(2)?,
        display_title: row.get(3)?,
        event: row.get(4)?,
        head_branch: row.get(5)?,
        head_sha: row.get(6)?,
        run_attempt: row.get(7)?,
        status: status.clone(),
        conclusion: row.get(9)?,
        html_url: row.get(10)?,
        resolution_status: row.get(11)?,
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
        attribution_source: row.get(15)?,
        attribution_reason: row.get(16)?,
        confidence: row.get(17)?,
    })
}

fn runs_for_track(conn: &Connection, track_id: i64, limit: i64) -> Result<Vec<WorkflowRunSummary>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT wr.run_id,mr.repo,wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,wr.created_at,wr.run_started_at,wr.updated_at,ra.source,ra.reason,ra.confidence
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN run_assignments ra ON ra.run_id=wr.run_id
         WHERE ra.track_id=? AND wr.ignored=0
         ORDER BY wr.created_at DESC LIMIT ?",
    )?;
    let rows = stmt.query_map(params![track_id, limit], |row| run_summary_from_row(row, now))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn unassigned_runs(conn: &Connection, limit: i64) -> Result<Vec<WorkflowRunSummary>> {
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT wr.run_id,mr.repo,wr.workflow_name,wr.display_title,wr.event,wr.head_branch,wr.head_sha,wr.run_attempt,wr.status,wr.conclusion,wr.html_url,wr.resolution_status,wr.created_at,wr.run_started_at,wr.updated_at,
                CASE WHEN wr.resolution_status='conflict' THEN 'explicit_conflict'
                     ELSE (SELECT re.signal_type FROM run_evidence re WHERE re.run_id=wr.run_id ORDER BY re.score DESC LIMIT 1) END,
                (SELECT 'Track Key 후보: ' || group_concat(track_key, ', ') FROM (SELECT DISTINCT re.track_key track_key FROM run_evidence re WHERE re.run_id=wr.run_id AND re.score>=90)),
                (SELECT MAX(re.score) FROM run_evidence re WHERE re.run_id=wr.run_id)
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         WHERE ra.run_id IS NULL AND wr.ignored=0
         ORDER BY CASE WHEN wr.status='completed' THEN 1 ELSE 0 END, wr.created_at DESC LIMIT ?",
    )?;
    let rows = stmt.query_map(params![limit], |row| run_summary_from_row(row, now))?;
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
    let repositories = list_repositories(&conn, false)?;
    let tracks = list_tracks(&conn, true)?;
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
    let unassigned_runs = unassigned_runs(&conn, 30)?;
    let unassigned_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workflow_runs wr LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id WHERE ra.run_id IS NULL AND wr.ignored=0 AND wr.status!='completed'",
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
        repositories,
        tracks: dashboard_tracks,
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

fn load_fingerprints(conn: &Connection, repository_id: i64) -> Result<Vec<Fingerprint>> {
    let mut stmt = conn.prepare(
        "SELECT wt.track_key,tf.signal_type,tf.pattern,tf.weight
         FROM track_fingerprints tf
         JOIN watch_tracks wt ON wt.id=tf.track_id
         WHERE tf.active=1 AND wt.active=1 AND (tf.repository_id IS NULL OR tf.repository_id=?)",
    )?;
    let rows = stmt.query_map(params![repository_id], |row| {
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

async fn resolve_run(
    client: &Client,
    repo: &str,
    run: &GithubRun,
    tracks: &[Track],
    fingerprints: &[Fingerprint],
    commit_cache: &mut HashMap<String, Option<String>>,
    pr_cache: &mut HashMap<String, Vec<GithubPull>>,
) -> Result<Resolution> {
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
                score: 100,
                value: format!("PR #{}", pr.number),
            });
        }
    }

    if let Some(message) = github_commit_message(client, repo, &run.head_sha, commit_cache).await {
        if let Some(key) = extract_track_trailer(&message) {
            evidence.push(Evidence {
                track_key: key,
                signal_type: "commit_marker".into(),
                score: 95,
                value: run.head_sha.clone(),
            });
        }
    }

    evidence.extend(fingerprint_evidence(fingerprints, run));

    let known: HashMap<&str, i64> = tracks.iter().map(|t| (t.track_key.as_str(), t.id)).collect();
    let explicit_keys: HashSet<String> = evidence
        .iter()
        .filter(|e| e.score >= 90 && known.contains_key(e.track_key.as_str()))
        .map(|e| e.track_key.clone())
        .collect();

    if explicit_keys.len() > 1 {
        return Ok(Resolution {
            status: "conflict".into(),
            track_id: None,
            confidence: None,
            source: Some("explicit_conflict".into()),
            reason: Some(format!(
                "명시적 Track Key가 충돌합니다: {}",
                explicit_keys.into_iter().collect::<Vec<_>>().join(", ")
            )),
            evidence,
        });
    }

    if let Some(key) = explicit_keys.iter().next() {
        let best = evidence
            .iter()
            .filter(|e| &e.track_key == key)
            .max_by_key(|e| e.score)
            .expect("explicit evidence");
        return Ok(Resolution {
            status: "assigned".into(),
            track_id: known.get(key.as_str()).copied(),
            confidence: Some(best.score),
            source: Some(best.signal_type.clone()),
            reason: Some(format!("{} → {}", best.signal_type, key)),
            evidence,
        });
    }

    let mut scores: HashMap<String, i64> = HashMap::new();
    for item in &evidence {
        if known.contains_key(item.track_key.as_str()) {
            *scores.entry(item.track_key.clone()).or_default() += item.score;
        }
    }
    let mut ranked: Vec<(String, i64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    if let Some((key, score)) = ranked.first() {
        let second = ranked.get(1).map(|v| v.1).unwrap_or(0);
        if *score >= 70 && *score - second >= 30 {
            return Ok(Resolution {
                status: "assigned".into(),
                track_id: known.get(key.as_str()).copied(),
                confidence: Some((*score).min(100)),
                source: Some("inference".into()),
                reason: Some(format!("복합 신호 {}점 (2위 {}점)", score, second)),
                evidence,
            });
        }
    }

    let unknown_explicit: Vec<String> = evidence
        .iter()
        .filter(|e| e.score >= 90 && !known.contains_key(e.track_key.as_str()))
        .map(|e| e.track_key.clone())
        .collect();
    let reason = if unknown_explicit.is_empty() {
        "확정 가능한 Track Key 근거가 없습니다.".to_string()
    } else {
        format!("등록되지 않은 Track Key 발견: {}", unknown_explicit.join(", "))
    };
    Ok(Resolution {
        status: "unassigned".into(),
        track_id: None,
        confidence: ranked.first().map(|v| v.1.min(100)),
        source: None,
        reason: Some(reason),
        evidence,
    })
}

fn persist_resolution(conn: &Connection, run_id: i64, resolution: &Resolution, now: &str) -> Result<()> {
    let manual: Option<i64> = conn
        .query_row(
            "SELECT manual FROM run_assignments WHERE run_id=?",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    if manual == Some(1) {
        return Ok(());
    }
    conn.execute("DELETE FROM run_evidence WHERE run_id=?", params![run_id])?;
    for item in &resolution.evidence {
        conn.execute(
            "INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) VALUES(?,?,?,?,?,?)",
            params![run_id, item.track_key, item.signal_type, item.score, item.value, now],
        )?;
    }
    conn.execute(
        "UPDATE workflow_runs SET resolution_status=? WHERE run_id=?",
        params![resolution.status, run_id],
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
    Ok(())
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
                    load_fingerprints(&conn, repository.id)?
                };
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
                        &tracks,
                        &fingerprints,
                        &mut commit_cache,
                        &mut pr_cache,
                    )
                    .await?;
                    let conn = db(state)?;
                    persist_resolution(&conn, run.id, &resolution, &now_str)?;
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
async fn poll_now(app: AppHandle) -> std::result::Result<Dashboard, String> {
    poll_all(&app).await.map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    build_dashboard(&state).map_err(|e| e.to_string())
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
        let now = Utc::now().to_rfc3339();
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE watch_tracks SET name=?,track_key=?,long_ci_minutes=?,active=1,updated_at=? WHERE id=?",
                params![name, track_key, input.long_ci_minutes, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 트랙을 찾지 못했습니다."));
            }
            id
        } else {
            conn.execute(
                "INSERT INTO watch_tracks(name,track_key,long_ci_minutes,active,created_at,updated_at) VALUES(?,?,?,1,?,?)",
                params![name, track_key, input.long_ci_minutes, now, now],
            )?;
            conn.last_insert_rowid()
        };
        Ok(id)
    })();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_track(id: i64, state: State<'_, AppState>) -> std::result::Result<(), String> {
    let conn = db(&state).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM watch_tracks WHERE id=?", params![id])
        .map(|_| ())
        .map_err(|e| e.to_string())
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
        let now = Utc::now().to_rfc3339();
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE monitored_repositories SET repo=?,enabled=?,updated_at=? WHERE id=?",
                params![repo, if input.enabled { 1 } else { 0 }, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 저장소를 찾지 못했습니다."));
            }
            id
        } else {
            conn.execute(
                "INSERT INTO monitored_repositories(repo,enabled,created_at,updated_at) VALUES(?,?,?,?)",
                params![repo, if input.enabled { 1 } else { 0 }, now, now],
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

#[tauri::command]
fn assign_run(
    run_id: i64,
    track_id: i64,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let result = (|| -> Result<()> {
        let conn = db(&state)?;
        let now = Utc::now().to_rfc3339();
        let workflow_name: String = conn.query_row(
            "SELECT workflow_name FROM workflow_runs WHERE run_id=?",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO run_assignments(run_id,track_id,confidence,source,reason,manual,assigned_at)
             VALUES(?,?,100,'manual','사용자 수동 귀속',1,?)
             ON CONFLICT(run_id) DO UPDATE SET track_id=excluded.track_id,confidence=100,source='manual',reason='사용자 수동 귀속',manual=1,assigned_at=excluded.assigned_at",
            params![run_id, track_id, now],
        )?;
        conn.execute(
            "UPDATE workflow_runs SET resolution_status='assigned' WHERE run_id=?",
            params![run_id],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO track_fingerprints(track_id,signal_type,pattern,repository_id,weight,learned_from_run_id,active,created_at)
             SELECT ?, 'workflow_name', ?, repository_id, 50, ?, 1, ? FROM workflow_runs WHERE run_id=?",
            params![track_id, workflow_name, run_id, now, run_id],
        )?;
        Ok(())
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
            poll_now,
            save_track,
            delete_track,
            save_repository,
            delete_repository,
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

    fn track(id: i64, key: &str) -> Track {
        Track {
            id,
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

    #[test]
    fn explicit_conflict_model_has_distinct_keys() {
        let tracks = vec![track(1, "ops"), track(2, "saju")];
        let known: HashSet<_> = tracks.iter().map(|t| t.track_key.as_str()).collect();
        assert!(known.contains("ops"));
        assert!(known.contains("saju"));
    }
}
