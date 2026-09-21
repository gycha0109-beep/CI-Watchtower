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
    repo: String,
    source_mode: String,
    branch: Option<String>,
    pr_number: Option<i64>,
    workflow_filter: Option<String>,
    long_ci_minutes: i64,
    archived: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackInput {
    id: Option<i64>,
    name: String,
    repo: String,
    source_mode: String,
    branch: Option<String>,
    pr_number: Option<i64>,
    workflow_filter: Option<String>,
    long_ci_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunSummary {
    id: i64,
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    created_at: String,
    run_started_at: Option<String>,
    updated_at: String,
    elapsed_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackStateView {
    track_id: i64,
    health: String,
    head_sha: Option<String>,
    pr_url: Option<String>,
    latest_run_url: Option<String>,
    checked_at: Option<String>,
    message: Option<String>,
    elapsed_seconds: i64,
    average_duration_seconds: Option<i64>,
    runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardTrack {
    track: Track,
    state: TrackStateView,
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
    congestion_level: String,
    token_configured: bool,
    settings: Settings,
    tracks: Vec<DashboardTrack>,
}

#[derive(Debug, Deserialize)]
struct GithubPull {
    head: GithubHead,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubHead {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GithubCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GithubRunsResponse {
    workflow_runs: Vec<GithubRun>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRun {
    id: i64,
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    created_at: String,
    run_started_at: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct SourceSnapshot {
    head_sha: String,
    pr_url: Option<String>,
    runs: Vec<GithubRun>,
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

        CREATE TABLE IF NOT EXISTS track_state (
          track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
          health TEXT NOT NULL DEFAULT 'waiting',
          head_sha TEXT,
          pr_url TEXT,
          latest_run_url TEXT,
          checked_at TEXT,
          message TEXT,
          elapsed_seconds INTEGER NOT NULL DEFAULT 0,
          runs_json TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS run_history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          run_id INTEGER NOT NULL,
          duration_seconds INTEGER NOT NULL,
          conclusion TEXT,
          completed_at TEXT NOT NULL,
          UNIQUE(track_id, run_id)
        );

        CREATE TABLE IF NOT EXISTS notified_events (
          track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          event_key TEXT NOT NULL,
          notified_at TEXT NOT NULL,
          PRIMARY KEY(track_id, event_key)
        );

        CREATE TABLE IF NOT EXISTS repo_activity (
          repo TEXT PRIMARY KEY,
          running_count INTEGER NOT NULL DEFAULT 0,
          queued_count INTEGER NOT NULL DEFAULT 0,
          checked_at TEXT NOT NULL
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
        .user_agent("ci-watchtower/0.1.0")
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
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(anyhow!("Repository 이름에 허용되지 않은 문자가 있습니다."));
    }
    Ok(())
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value).ok().map(|v| v.with_timezone(&Utc))
}

fn run_elapsed_seconds(run: &GithubRun, now: DateTime<Utc>) -> i64 {
    let start = run
        .run_started_at
        .as_deref()
        .and_then(parse_time)
        .or_else(|| parse_time(&run.created_at));
    let end = if run.status == "completed" {
        parse_time(&run.updated_at).unwrap_or(now)
    } else {
        now
    };
    start.map(|s| (end - s).num_seconds().max(0)).unwrap_or(0)
}

fn to_summary(run: &GithubRun, now: DateTime<Utc>) -> WorkflowRunSummary {
    WorkflowRunSummary {
        id: run.id,
        name: run.name.clone(),
        status: run.status.clone(),
        conclusion: run.conclusion.clone(),
        html_url: run.html_url.clone(),
        created_at: run.created_at.clone(),
        run_started_at: run.run_started_at.clone(),
        updated_at: run.updated_at.clone(),
        elapsed_seconds: run_elapsed_seconds(run, now),
    }
}

fn apply_workflow_filter(runs: &[GithubRun], filter: Option<&str>) -> Vec<GithubRun> {
    let Some(filter) = filter.map(str::trim).filter(|v| !v.is_empty()) else {
        return runs.to_vec();
    };
    let terms: Vec<String> = filter
        .split(',')
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .collect();
    if terms.is_empty() {
        return runs.to_vec();
    }
    runs.iter()
        .filter(|r| {
            let name = r.name.to_lowercase();
            terms.iter().any(|term| name.contains(term))
        })
        .cloned()
        .collect()
}

fn calculate_health(runs: &[GithubRun]) -> String {
    if runs.is_empty() {
        return "waiting".into();
    }
    if runs.iter().any(|r| r.status == "in_progress") {
        return "running".into();
    }
    if runs.iter().any(|r| r.status == "queued" || r.status == "requested" || r.status == "pending") {
        return "queued".into();
    }
    let failure_like = [
        "failure",
        "cancelled",
        "timed_out",
        "action_required",
        "startup_failure",
        "stale",
    ];
    if runs.iter().any(|r| {
        r.conclusion
            .as_deref()
            .map(|c| failure_like.contains(&c))
            .unwrap_or(false)
    }) {
        return "red".into();
    }
    if runs.iter().all(|r| r.status == "completed" && r.conclusion.as_deref() == Some("success")) {
        return "green".into();
    }
    if runs.iter().all(|r| r.status == "completed") {
        return "completed_other".into();
    }
    "waiting".into()
}

async fn github_repo_activity(client: &Client, repo: &str) -> Result<(i64, i64)> {
    let url = format!("https://api.github.com/repos/{}/actions/runs?per_page=100", repo);
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("GitHub repository Actions 조회 실패: HTTP {}", response.status()));
    }
    let data: GithubRunsResponse = response.json().await?;
    let running = data.workflow_runs.iter().filter(|r| r.status == "in_progress").count() as i64;
    let queued = data.workflow_runs.iter().filter(|r| matches!(r.status.as_str(), "queued" | "requested" | "pending" | "waiting")).count() as i64;
    Ok((running, queued))
}

async fn github_source_snapshot(client: &Client, track: &Track) -> Result<SourceSnapshot> {
    let (head_sha, pr_url) = if track.source_mode == "pr" {
        let pr = track.pr_number.ok_or_else(|| anyhow!("PR 번호가 없습니다."))?;
        let url = format!("https://api.github.com/repos/{}/pulls/{}", track.repo, pr);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!("GitHub PR 조회 실패: HTTP {}", response.status()));
        }
        let data: GithubPull = response.json().await?;
        (data.head.sha, Some(data.html_url))
    } else {
        let branch = track.branch.as_deref().ok_or_else(|| anyhow!("Branch가 없습니다."))?;
        let url = format!(
            "https://api.github.com/repos/{}/commits/{}",
            track.repo,
            urlencoding::encode(branch)
        );
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!("GitHub Branch 조회 실패: HTTP {}", response.status()));
        }
        let data: GithubCommit = response.json().await?;
        (data.sha, None)
    };

    let runs_url = format!(
        "https://api.github.com/repos/{}/actions/runs?head_sha={}&per_page=100",
        track.repo,
        urlencoding::encode(&head_sha)
    );
    let response = client.get(runs_url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("GitHub Actions 조회 실패: HTTP {}", response.status()));
    }
    let data: GithubRunsResponse = response.json().await?;
    Ok(SourceSnapshot { head_sha, pr_url, runs: data.workflow_runs })
}

fn source_cache_key(track: &Track) -> String {
    match track.source_mode.as_str() {
        "pr" => format!("{}|pr|{}", track.repo, track.pr_number.unwrap_or_default()),
        _ => format!("{}|branch|{}", track.repo, track.branch.as_deref().unwrap_or_default()),
    }
}

fn send_notification(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn mark_notified(conn: &Connection, track_id: i64, key: &str, now: &str) -> Result<bool> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO notified_events(track_id, event_key, notified_at) VALUES(?,?,?)",
        params![track_id, key, now],
    )?;
    Ok(changed > 0)
}

fn load_settings(conn: &Connection) -> Result<Settings> {
    conn.query_row(
        "SELECT queue_congestion_threshold, active_poll_seconds, idle_poll_seconds, auto_archive_completed FROM app_settings WHERE id=1",
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

fn list_tracks(conn: &Connection, include_archived: bool) -> Result<Vec<Track>> {
    let sql = if include_archived {
        "SELECT id,name,repo,source_mode,branch,pr_number,workflow_filter,long_ci_minutes,archived,created_at,updated_at FROM tracks ORDER BY id DESC"
    } else {
        "SELECT id,name,repo,source_mode,branch,pr_number,workflow_filter,long_ci_minutes,archived,created_at,updated_at FROM tracks WHERE archived=0 ORDER BY id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Track {
            id: row.get(0)?,
            name: row.get(1)?,
            repo: row.get(2)?,
            source_mode: row.get(3)?,
            branch: row.get(4)?,
            pr_number: row.get(5)?,
            workflow_filter: row.get(6)?,
            long_ci_minutes: row.get(7)?,
            archived: row.get::<_, i64>(8)? != 0,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn average_duration(conn: &Connection, track_id: i64) -> Result<Option<i64>> {
    let avg: Option<f64> = conn
        .query_row(
            "SELECT AVG(duration_seconds) FROM (SELECT duration_seconds FROM run_history WHERE track_id=? ORDER BY completed_at DESC LIMIT 20)",
            params![track_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(avg.map(|v| v.round() as i64))
}

fn state_for_track(conn: &Connection, track_id: i64) -> Result<TrackStateView> {
    let state = conn
        .query_row(
            "SELECT health,head_sha,pr_url,latest_run_url,checked_at,message,elapsed_seconds,runs_json FROM track_state WHERE track_id=?",
            params![track_id],
            |row| {
                let runs_json: String = row.get(7)?;
                let runs: Vec<WorkflowRunSummary> = serde_json::from_str(&runs_json).unwrap_or_default();
                Ok(TrackStateView {
                    track_id,
                    health: row.get(0)?,
                    head_sha: row.get(1)?,
                    pr_url: row.get(2)?,
                    latest_run_url: row.get(3)?,
                    checked_at: row.get(4)?,
                    message: row.get(5)?,
                    elapsed_seconds: row.get(6)?,
                    average_duration_seconds: None,
                    runs,
                })
            },
        )
        .optional()?;

    let mut state = state.unwrap_or(TrackStateView {
        track_id,
        health: "waiting".into(),
        head_sha: None,
        pr_url: None,
        latest_run_url: None,
        checked_at: None,
        message: None,
        elapsed_seconds: 0,
        average_duration_seconds: None,
        runs: vec![],
    });
    state.average_duration_seconds = average_duration(conn, track_id)?;
    Ok(state)
}

fn build_dashboard(state: &AppState) -> Result<Dashboard> {
    let conn = db(state)?;
    let settings = load_settings(&conn)?;
    let tracks = list_tracks(&conn, false)?;
    let mut dashboard_tracks = Vec::with_capacity(tracks.len());
    for track in tracks {
        let st = state_for_track(&conn, track.id)?;
        dashboard_tracks.push(DashboardTrack { track, state: st });
    }

    let (running_count, queued_count): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(running_count),0), COALESCE(SUM(queued_count),0) FROM repo_activity WHERE repo IN (SELECT DISTINCT repo FROM tracks WHERE archived=0)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
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
        congestion_level: congestion_level.into(),
        token_configured: token_configured(),
        settings,
        tracks: dashboard_tracks,
    })
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
    let tracks = {
        let conn = db(state)?;
        list_tracks(&conn, false)?
    };
    if tracks.is_empty() {
        return Ok(());
    }

    let mut cache: HashMap<String, std::result::Result<SourceSnapshot, String>> = HashMap::new();
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let repos: HashSet<String> = tracks.iter().map(|t| t.repo.clone()).collect();
    for repo in repos {
        if let Ok((running_count, queued_count)) = github_repo_activity(&client, &repo).await {
            let conn = db(state)?;
            conn.execute(
                "INSERT INTO repo_activity(repo,running_count,queued_count,checked_at) VALUES(?,?,?,?) ON CONFLICT(repo) DO UPDATE SET running_count=excluded.running_count,queued_count=excluded.queued_count,checked_at=excluded.checked_at",
                params![repo, running_count, queued_count, now_str],
            )?;
        }
    }

    for track in &tracks {
        let cache_key = source_cache_key(track);
        if !cache.contains_key(&cache_key) {
            let fetched = github_source_snapshot(&client, track)
                .await
                .map_err(|e| e.to_string());
            cache.insert(cache_key.clone(), fetched);
        }

        let snapshot = cache.get(&cache_key).expect("cache inserted");
        let conn = db(state)?;

        match snapshot {
            Err(message) => {
                conn.execute(
                    "INSERT INTO track_state(track_id,health,checked_at,message,runs_json) VALUES(?, 'error', ?, ?, '[]') ON CONFLICT(track_id) DO UPDATE SET health='error', checked_at=excluded.checked_at, message=excluded.message, runs_json='[]'",
                    params![track.id, now_str, message],
                )?;
            }
            Ok(snapshot) => {
                let runs = apply_workflow_filter(&snapshot.runs, track.workflow_filter.as_deref());
                let health = calculate_health(&runs);
                let summaries: Vec<_> = runs.iter().map(|r| to_summary(r, now)).collect();
                let elapsed = summaries
                    .iter()
                    .filter(|r| r.status == "in_progress" || r.status == "queued" || r.status == "requested" || r.status == "pending")
                    .map(|r| r.elapsed_seconds)
                    .max()
                    .unwrap_or_else(|| summaries.iter().map(|r| r.elapsed_seconds).max().unwrap_or(0));
                let latest_url = summaries.first().map(|r| r.html_url.clone());
                let message = match health.as_str() {
                    "green" => Some("현재 SHA의 대상 workflow가 모두 success로 완료되었습니다.".to_string()),
                    "red" => Some("현재 SHA에서 실패성 conclusion이 확인되었습니다.".to_string()),
                    "completed_other" => Some("모든 workflow가 끝났지만 success 이외의 conclusion이 포함되어 있습니다.".to_string()),
                    "running" => Some("CI가 실행 중입니다.".to_string()),
                    "queued" => Some("CI가 GitHub Actions queue에서 대기 중입니다.".to_string()),
                    _ => Some("현재 SHA에서 대상 workflow run을 기다리는 중입니다.".to_string()),
                };

                conn.execute(
                    "INSERT INTO track_state(track_id,health,head_sha,pr_url,latest_run_url,checked_at,message,elapsed_seconds,runs_json) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(track_id) DO UPDATE SET health=excluded.health,head_sha=excluded.head_sha,pr_url=excluded.pr_url,latest_run_url=excluded.latest_run_url,checked_at=excluded.checked_at,message=excluded.message,elapsed_seconds=excluded.elapsed_seconds,runs_json=excluded.runs_json",
                    params![
                        track.id,
                        health,
                        snapshot.head_sha,
                        snapshot.pr_url,
                        latest_url,
                        now_str,
                        message,
                        elapsed,
                        serde_json::to_string(&summaries)?
                    ],
                )?;

                for run in &runs {
                    if run.status == "completed" {
                        let duration = run_elapsed_seconds(run, now);
                        conn.execute(
                            "INSERT OR IGNORE INTO run_history(track_id,run_id,duration_seconds,conclusion,completed_at) VALUES(?,?,?,?,?)",
                            params![track.id, run.id, duration, run.conclusion, run.updated_at],
                        )?;
                    }
                }

                if health == "green" || health == "red" || health == "completed_other" {
                    let event_key = format!("complete:{}:{}", snapshot.head_sha, health);
                    if mark_notified(&conn, track.id, &event_key, &now_str)? {
                        let title = match health.as_str() {
                            "green" => format!("[{}] CI 완료 — GREEN", track.name),
                            "red" => format!("[{}] CI 완료 — RED", track.name),
                            _ => format!("[{}] CI 완료", track.name),
                        };
                        let body = format!("{} · {}", track.repo, message.clone().unwrap_or_default());
                        send_notification(app, &title, &body);
                    }
                }

                for run in &runs {
                    if run.status == "in_progress" {
                        let threshold_seconds = track.long_ci_minutes.saturating_mul(60);
                        let elapsed = run_elapsed_seconds(run, now);
                        if elapsed >= threshold_seconds {
                            let event_key = format!("long:{}:{}:{}", snapshot.head_sha, run.id, track.long_ci_minutes);
                            if mark_notified(&conn, track.id, &event_key, &now_str)? {
                                send_notification(
                                    app,
                                    &format!("[{}] 장기 CI 감지", track.name),
                                    &format!("{}이(가) 사용자 기준 {}분을 초과했습니다.", run.name, track.long_ci_minutes),
                                );
                            }
                        }
                    }
                }

                let settings = load_settings(&conn)?;
                if settings.auto_archive_completed && health == "green" {
                    conn.execute("UPDATE tracks SET archived=1, updated_at=? WHERE id=?", params![now_str, track.id])?;
                }
            }
        }
    }

    let dashboard = build_dashboard(state)?;
    let conn = db(state)?;
    let was_congested: bool = conn.query_row(
        "SELECT queue_congested FROM app_settings WHERE id=1",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    let now_congested = dashboard.queued_count >= dashboard.settings.queue_congestion_threshold as usize;
    if now_congested && !was_congested {
        send_notification(
            app,
            "GitHub Actions Queue 혼잡",
            &format!(
                "Queued {} · Running {} · 설정 기준 {}",
                dashboard.queued_count, dashboard.running_count, dashboard.settings.queue_congestion_threshold
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
        let repo = input.repo.trim();
        if name.is_empty() {
            return Err(anyhow!("트랙 이름을 입력하십시오."));
        }
        validate_repo(repo)?;
        if input.long_ci_minutes <= 0 || input.long_ci_minutes > 10080 {
            return Err(anyhow!("장기 CI 기준시간은 1~10080분 사이로 입력하십시오."));
        }
        match input.source_mode.as_str() {
            "branch" if input.branch.as_deref().map(str::trim).filter(|v| !v.is_empty()).is_some() => {}
            "pr" if input.pr_number.unwrap_or(0) > 0 => {}
            _ => return Err(anyhow!("Branch 또는 PR 정보를 올바르게 입력하십시오.")),
        }
        let conn = db(&state)?;
        let now = Utc::now().to_rfc3339();
        let branch = if input.source_mode == "branch" { input.branch.map(|v| v.trim().to_string()) } else { None };
        let pr_number = if input.source_mode == "pr" { input.pr_number } else { None };
        let filter = input.workflow_filter.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let id = if let Some(id) = input.id {
            conn.execute(
                "UPDATE tracks SET name=?,repo=?,source_mode=?,branch=?,pr_number=?,workflow_filter=?,long_ci_minutes=?,archived=0,updated_at=? WHERE id=?",
                params![name, repo, input.source_mode, branch, pr_number, filter, input.long_ci_minutes, now, id],
            )?;
            if conn.changes() == 0 {
                return Err(anyhow!("수정할 트랙을 찾지 못했습니다."));
            }
            conn.execute("DELETE FROM track_state WHERE track_id=?", params![id])?;
            id
        } else {
            conn.execute(
                "INSERT INTO tracks(name,repo,source_mode,branch,pr_number,workflow_filter,long_ci_minutes,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                params![name, repo, input.source_mode, branch, pr_number, filter, input.long_ci_minutes, now, now],
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
    conn.execute("DELETE FROM tracks WHERE id=?", params![id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn unarchive_all(state: State<'_, AppState>) -> std::result::Result<(), String> {
    let conn = db(&state).map_err(|e| e.to_string())?;
    conn.execute("UPDATE tracks SET archived=0, updated_at=? WHERE archived=1", params![Utc::now().to_rfc3339()])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(settings: Settings, state: State<'_, AppState>) -> std::result::Result<(), String> {
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
    keyring_entry()
        .and_then(|entry| entry.set_password(token).map_err(|e| anyhow!(e.to_string())))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_github_token() -> std::result::Result<(), String> {
    match keyring_entry() {
        Ok(entry) => match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(_) => Ok(()),
        },
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
            let (seconds, has_active) = {
                let state = handle.state::<AppState>();
                match build_dashboard(&state) {
                    Ok(dashboard) => {
                        let active = dashboard.tracks.iter().any(|item| {
                            matches!(item.state.health.as_str(), "running" | "queued" | "waiting" | "error")
                        });
                        let sec = if active {
                            dashboard.settings.active_poll_seconds
                        } else {
                            dashboard.settings.idle_poll_seconds
                        };
                        (sec.max(10) as u64, active)
                    }
                    Err(_) => (DEFAULT_IDLE_POLL_SECONDS as u64, false),
                }
            };
            let _ = has_active;
            tokio::time::sleep(Duration::from_secs(seconds)).await;
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
            unarchive_all,
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

    fn run(id: i64, status: &str, conclusion: Option<&str>, name: &str) -> GithubRun {
        GithubRun {
            id,
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            html_url: format!("https://github.com/example/repo/actions/runs/{id}"),
            created_at: "2026-09-21T12:00:00Z".into(),
            run_started_at: Some("2026-09-21T12:00:05Z".into()),
            updated_at: "2026-09-21T12:02:05Z".into(),
        }
    }

    #[test]
    fn all_success_is_green() {
        let runs = vec![
            run(1, "completed", Some("success"), "CI"),
            run(2, "completed", Some("success"), "Integration"),
        ];
        assert_eq!(calculate_health(&runs), "green");
    }

    #[test]
    fn one_failure_is_red() {
        let runs = vec![
            run(1, "completed", Some("success"), "CI"),
            run(2, "completed", Some("failure"), "Integration"),
        ];
        assert_eq!(calculate_health(&runs), "red");
    }

    #[test]
    fn running_takes_precedence_until_terminal() {
        let runs = vec![
            run(1, "in_progress", None, "CI"),
            run(2, "completed", Some("failure"), "Integration"),
        ];
        assert_eq!(calculate_health(&runs), "running");
    }

    #[test]
    fn workflow_filter_accepts_comma_separated_terms() {
        let runs = vec![
            run(1, "completed", Some("success"), "Lint"),
            run(2, "completed", Some("success"), "Integration Tests"),
            run(3, "completed", Some("success"), "Release"),
        ];
        let filtered = apply_workflow_filter(&runs, Some("CI, integration"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Integration Tests");
    }

    #[test]
    fn repository_format_is_bounded() {
        assert!(validate_repo("gycha0109-beep/Saju").is_ok());
        assert!(validate_repo("bad/repo/extra").is_err());
        assert!(validate_repo("https://github.com/a/b").is_err());
    }
}
