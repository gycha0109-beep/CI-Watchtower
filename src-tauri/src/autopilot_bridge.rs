//! Versioned, read-only localhost status bridge. Never exposes keyring secrets or a writable DB.
use super::*;
use axum::{
    extract::{Path as HttpPath, Query, State as HttpState},
    http::{header::{AUTHORIZATION, CACHE_CONTROL, HOST, ORIGIN}, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rand::{rngs::OsRng, RngCore};
use rusqlite::OpenFlags;
use serde_json::{json, Value};
use std::sync::Arc;
use subtle::ConstantTimeEq;

const PORT: u16 = 48765;
const CONTRACT_VERSION: &str = "1.0";
const BRIDGE_ACCOUNT: &str = "development-autopilot-bridge-v1";
const DEFAULT_LIMIT: i64 = 30;

#[derive(Clone)]
struct BridgeState {
    db_path: PathBuf,
    token: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct Scope {
    project_id: Option<i64>,
    repository_id: Option<i64>,
    track_key: Option<String>,
    head_sha: Option<String>,
    attempt: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryRepository {
    id: i64,
    project_id: i64,
    name: String,
    enabled: bool,
    last_poll_attempt_at: Option<String>,
    last_successful_poll_at: Option<String>,
    poll_error_code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryTrack {
    id: i64,
    project_id: i64,
    canonical_track_key: String,
    name: String,
    active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryProject {
    id: i64,
    key: String,
    name: String,
    active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRun {
    run_id: i64,
    project_id: i64,
    project_key: String,
    repository_id: i64,
    repository: String,
    repository_enabled: bool,
    workflow_id: i64,
    workflow_name: String,
    workflow_path: Option<String>,
    head_sha: String,
    head_branch: Option<String>,
    run_attempt: i64,
    latest_known_attempt: i64,
    status: String,
    conclusion: Option<String>,
    ci_state: &'static str,
    decision_state: &'static str,
    evidence_state: &'static str,
    responsibility_kind: &'static str,
    technical_responsibility: Option<String>,
    resolution_status: String,
    work_track_id: Option<i64>,
    work_track_key: Option<String>,
    work_track_active: bool,
    work_track_source: Option<String>,
    assignment_track_id: Option<i64>,
    assignment_track_key: Option<String>,
    assignment_source: Option<String>,
    attribution_conflict: bool,
    created_at: String,
    updated_at: String,
    last_seen_at: String,
    last_poll_attempt_at: Option<String>,
    last_successful_poll_at: Option<String>,
    poll_error_code: Option<&'static str>,
    observed_at: String,
    freshness_limit_seconds: i64,
    target_verified: bool,
    safe_to_advance: bool,
}

fn api_error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"contractVersion": CONTRACT_VERSION, "error": {"code":code}}))).into_response()
}

fn response(value: Value) -> Response {
    Json(json!({"contractVersion": CONTRACT_VERSION, "data": value})).into_response()
}

fn read_connection(path: &std::path::Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?)
}

fn poll_error_code(message: Option<&str>) -> Option<&'static str> {
    message.map(|message| {
        if message.contains("401") || message.contains("403") { "GITHUB_AUTH_OR_PERMISSION" }
        else if message.contains("429") || message.contains("rate limit") { "GITHUB_RATE_LIMIT" }
        else { "GITHUB_POLL_FAILED" }
    })
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 100
        && key.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_scope(scope: &Scope, require_repo: bool) -> std::result::Result<(), Response> {
    if require_repo && scope.repository_id.is_none() {
        return Err(api_error(StatusCode::BAD_REQUEST, "REPOSITORY_SCOPE_REQUIRED"));
    }
    if scope.project_id.is_some_and(|v| v <= 0)
        || scope.repository_id.is_some_and(|v| v <= 0)
        || scope.attempt.is_some_and(|v| v <= 0)
        || scope.limit.is_some_and(|v| !(1..=100).contains(&v))
        || scope.track_key.as_deref().is_some_and(|v| !valid_key(v))
        || scope.head_sha.as_deref().is_some_and(|v|
            v.len() != 40 || !v.bytes().all(|b| b.is_ascii_hexdigit())
        )
    {
        return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_SCOPE"));
    }
    Ok(())
}

fn ct_token_eq(actual: &str, expected: &str) -> bool {
    let a = actual.as_bytes();
    let b = expected.as_bytes();
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

async fn authorize(
    HttpState(state): HttpState<Arc<BridgeState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let headers = request.headers();
    // Reject browser origins and DNS-rebinding hosts, including preflight requests.
    let expected_host = format!("127.0.0.1:{PORT}");
    let host = headers.get(HOST).and_then(|v| v.to_str().ok());
    if headers.contains_key(ORIGIN) || host != Some(expected_host.as_str()) {
        return api_error(StatusCode::FORBIDDEN, "LOCAL_CLIENT_ONLY");
    }
    let authorized = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| ct_token_eq(token, &state.token));
    if !authorized {
        return api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    response.headers_mut().insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}

fn registry(conn: &Connection, scope: &Scope) -> Result<Value> {
    let projects = list_projects(conn, false)?.into_iter()
        .filter(|p| scope.project_id.is_none_or(|id| id == p.id))
        .map(|p| RegistryProject { id: p.id, key: p.project_key, name: p.name, active: p.active })
        .collect::<Vec<_>>();
    let repositories = list_repositories(conn, false)?.into_iter()
        .filter(|r| scope.project_id.is_none_or(|id| id == r.project_id))
        .filter(|r| scope.repository_id.is_none_or(|id| id == r.id))
        .map(|r| {
            let success = conn.query_row(
                "SELECT last_successful_poll_at FROM monitored_repositories WHERE id=?",
                params![r.id], |row| row.get::<_, Option<String>>(0)
            )?;
            Ok(RegistryRepository {
                id: r.id, project_id: r.project_id, name: r.repo, enabled: r.enabled,
                last_poll_attempt_at: r.last_polled_at,
                last_successful_poll_at: success, poll_error_code: poll_error_code(r.last_error.as_deref()),
            })
        }).collect::<rusqlite::Result<Vec<_>>>()?;
    let tracks = list_tracks(conn, false)?.into_iter()
        .filter(|t| scope.project_id.is_none_or(|id| id == t.project_id))
        .map(|t| RegistryTrack {
            id:t.id, project_id:t.project_id, canonical_track_key:t.track_key,
            name:t.name, active:t.active,
        }).collect::<Vec<_>>();
    Ok(json!({"projects":projects,"repositories":repositories,"tracks":tracks}))
}

fn ci_state(status: &str, conclusion: Option<&str>) -> &'static str {
    match status {
        "in_progress" => "RUNNING",
        "queued" | "requested" | "pending" | "waiting" => "QUEUED",
        "completed" => match conclusion {
            Some("success") => "SUCCESS",
            Some("failure" | "timed_out" | "startup_failure") => "FAILURE",
            Some(_) => "OTHER_TERMINAL",
            None => "UNKNOWN",
        },
        _ => "UNKNOWN",
    }
}

fn freshness(
    now: DateTime<Utc>,
    enabled: bool,
    last_success: Option<&str>,
    last_error: Option<&str>,
    last_seen: &str,
    limit_seconds: i64,
    latest_attempt_matches: bool,
) -> &'static str {
    if !enabled { return "UNKNOWN"; }
    if last_error.is_some() { return "POLL_ERROR"; }
    let parse = |s: &str| DateTime::parse_from_rfc3339(s).ok().map(|v| v.with_timezone(&Utc));
    let Some(success) = last_success.and_then(parse) else { return "UNKNOWN"; };
    let Some(seen) = parse(last_seen) else { return "UNKNOWN"; };
    if !latest_attempt_matches || success > now + chrono::Duration::seconds(10)
        || seen > now + chrono::Duration::seconds(10)
        || (now-success).num_seconds() > limit_seconds
        || (now-seen).num_seconds() > limit_seconds {
        return "STALE";
    }
    "FRESH"
}

fn query_runs(conn: &Connection, scope: &Scope, exact_run_id: Option<i64>) -> Result<Vec<BridgeRun>> {
    let now = Utc::now();
    let (active, idle): (i64,i64) = conn.query_row(
        "SELECT active_poll_seconds,idle_poll_seconds FROM app_settings WHERE id=1",
        [], |row| Ok((row.get(0)?,row.get(1)?))
    )?;
    let freshness_limit_seconds = (active.max(idle).max(60) * 3).clamp(180, 3600);
    let limit = if exact_run_id.is_some() { 1 } else { scope.limit.unwrap_or(DEFAULT_LIMIT) };
    let mut stmt = conn.prepare(
        "SELECT wr.run_id,mr.project_id,p.project_key,mr.id,mr.repo,mr.enabled,
                wr.workflow_id,wr.workflow_name,wr.workflow_path,wr.head_sha,wr.head_branch,
                wr.run_attempt,wr.status,wr.conclusion,wr.resolution_status,
                wr.created_at,wr.updated_at,wr.last_seen_at,
                mr.last_polled_at,mr.last_successful_poll_at,mr.last_error,
                ra.track_id,assigned.track_key,ra.source,ra.manual,
                assoc.track_id,work.track_key,assoc.source,
                MAX(wr.run_attempt,COALESCE((SELECT MAX(attempt.run_attempt) FROM run_attempts attempt
                          WHERE attempt.run_id=wr.run_id),0)),
                COALESCE(CASE WHEN ra.manual=1 THEN assigned.active
                              WHEN assoc.run_id IS NOT NULL THEN work.active
                              ELSE assigned.active END,0)
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN projects p ON p.id=mr.project_id
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         LEFT JOIN watch_tracks assigned ON assigned.id=ra.track_id
         LEFT JOIN run_track_associations assoc ON assoc.run_id=wr.run_id
         LEFT JOIN watch_tracks work ON work.id=assoc.track_id
         WHERE wr.ignored=0
           AND (?1 IS NULL OR mr.id=?1)
           AND (?2 IS NULL OR mr.project_id=?2)
           AND (?3 IS NULL OR wr.run_id=?3)
           AND (?4 IS NULL OR wr.head_sha=?4)
           AND (?5 IS NULL OR
                (ra.manual=1 AND assigned.track_key=?5)
                OR (ra.manual IS NOT 1 AND work.track_key=?5)
                OR (assoc.run_id IS NULL AND assigned.track_key=?5))
         ORDER BY wr.created_at DESC,wr.run_id DESC
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![scope.repository_id, scope.project_id, exact_run_id, scope.head_sha, scope.track_key, limit],
        |r| {
            Ok((
                r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,
                r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,
                r.get::<_,i64>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<String>>(8)?,
                r.get::<_,String>(9)?,r.get::<_,Option<String>>(10)?,r.get::<_,i64>(11)?,
                r.get::<_,String>(12)?,r.get::<_,Option<String>>(13)?,r.get::<_,String>(14)?,
                r.get::<_,String>(15)?,r.get::<_,String>(16)?,r.get::<_,String>(17)?,
                r.get::<_,Option<String>>(18)?,r.get::<_,Option<String>>(19)?,
                r.get::<_,Option<String>>(20)?,
                r.get::<_,Option<i64>>(21)?,r.get::<_,Option<String>>(22)?,
                r.get::<_,Option<String>>(23)?,r.get::<_,Option<i64>>(24)?,
                r.get::<_,Option<i64>>(25)?,r.get::<_,Option<String>>(26)?,
                r.get::<_,Option<String>>(27)?,r.get::<_,i64>(28)?,r.get::<_,i64>(29)?,
            ))
        }
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id,project_id,project_key,repository_id,repository,enabled,workflow_id,
            workflow_name,workflow_path,head_sha,head_branch,run_attempt,status,conclusion,
            resolution_status,created_at,updated_at,last_seen_at,last_polled_at,
            last_success,last_error,assigned_id,assigned_key,assignment_source,manual,
            association_id,association_key,association_source,latest_known_attempt,work_track_active) = row?;
        let manual = manual == Some(1);
        let work_track_id = if manual { assigned_id } else { association_id.or(assigned_id) };
        let work_track_key = if manual { assigned_key.clone() } else { association_key.or(assigned_key.clone()) };
        let work_track_source = if manual { assignment_source.clone() } else { association_source.or(assignment_source.clone()) };
        let snapshot = producer_sync::load_snapshot(conn, repository_id)?;
        let producer = snapshot.and_then(|snapshot| snapshot.workflows.into_iter().find(|w|
            workflow_path.as_deref() == Some(w.path.as_str()) || w.name == workflow_name
        ));
        let rules_project: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_workflow_rules
              WHERE active=1 AND project_id=? AND workflow_name=?
                AND (repository_id IS NULL OR repository_id=?))",
            params![project_id,workflow_name,repository_id],
            |r| r.get(0),
        )?;
        let rules_dynamic: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dynamic_workflow_rules
              WHERE active=1 AND project_id=? AND workflow_name=?
                AND (repository_id IS NULL OR repository_id=?))",
            params![project_id,workflow_name,repository_id],
            |r| r.get(0),
        )?;
        let responsibility_kind =
            if rules_project != 0 || resolution_status == "project"
               || producer.as_ref().is_some_and(|p| p.binding == "unassigned-by-design") { "project-wide" }
            else if rules_dynamic != 0 ||
                producer.as_ref().is_some_and(|p| p.binding == "dynamic-by-run") { "dynamic" }
            else if producer.as_ref().is_some_and(|p| p.binding.starts_with("static:")) { "track-owned" }
            else { "unknown" };
        let evidence_state = freshness(
            now,enabled!=0,last_success.as_deref(),last_error.as_deref(),
            &last_seen_at,freshness_limit_seconds,latest_known_attempt == run_attempt,
        );
        let ci = ci_state(&status, conclusion.as_deref());
        let conflict = resolution_status == "conflict";
        let decision_state = if conflict { "CONFLICT" }
            else if work_track_id.is_some() && work_track_active == 0 { "UNKNOWN" }
            else if evidence_state != "FRESH" { evidence_state }
            else { ci };
        out.push(BridgeRun {
            run_id,project_id,project_key,repository_id,repository,repository_enabled:enabled!=0,
            workflow_id,workflow_name,workflow_path,head_sha,head_branch,run_attempt,
            latest_known_attempt,status,conclusion,ci_state:ci,decision_state,evidence_state,
            responsibility_kind,technical_responsibility:producer.map(|p|p.responsibility),
            resolution_status,work_track_id,work_track_key,work_track_active:work_track_active != 0,work_track_source,
            assignment_track_id:assigned_id,assignment_track_key:assigned_key,
            assignment_source,attribution_conflict:conflict,
            created_at,updated_at,last_seen_at,last_poll_attempt_at:last_polled_at,
            last_successful_poll_at:last_success,poll_error_code:poll_error_code(last_error.as_deref()),
            observed_at:now.to_rfc3339(),freshness_limit_seconds,
            target_verified:scope.head_sha.is_some() && scope.attempt == Some(run_attempt)
                && latest_known_attempt == run_attempt,
            safe_to_advance:false,
        });
    }
    Ok(out)
}

async fn health() -> Response {
    response(json!({"bridge":"connected","transport":"localhost","version":CONTRACT_VERSION}))
}

async fn registry_endpoint(
    HttpState(state): HttpState<Arc<BridgeState>>, Query(scope): Query<Scope>,
) -> Response {
    if let Err(err) = validate_scope(&scope, false) { return err; }
    match read_connection(&state.db_path).and_then(|c| registry(&c,&scope)) {
        Ok(result) => response(result),
        Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_UNAVAILABLE"),
    }
}

async fn runs_endpoint(
    HttpState(state): HttpState<Arc<BridgeState>>, Query(scope): Query<Scope>,
) -> Response {
    if let Err(err) = validate_scope(&scope, true) { return err; }
    match read_connection(&state.db_path).and_then(|c| query_runs(&c,&scope,None)) {
        Ok(runs) => response(json!({"runs":runs,"completeCoverage":false})),
        Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_UNAVAILABLE"),
    }
}

async fn single_run_endpoint(
    HttpPath(run_id): HttpPath<i64>,
    HttpState(state): HttpState<Arc<BridgeState>>, Query(scope): Query<Scope>,
) -> Response {
    if run_id <= 0 { return api_error(StatusCode::BAD_REQUEST, "INVALID_RUN_ID"); }
    if let Err(err) = validate_scope(&scope, false) { return err; }
    // Select the exact ID first, then compare expected SHA/attempt. A mismatch is
    // a conflict, not an absent run or permission to reuse an older GREEN.
    let mut lookup_scope = scope.clone();
    lookup_scope.head_sha = None;
    match read_connection(&state.db_path).and_then(|c| query_runs(&c,&lookup_scope,Some(run_id))) {
        Ok(mut runs) => {
            let Some(mut run) = runs.pop() else { return api_error(StatusCode::NOT_FOUND, "RUN_NOT_FOUND"); };
            if scope.head_sha.as_ref().is_some_and(|sha| sha != &run.head_sha) {
                return api_error(StatusCode::CONFLICT, "HEAD_SHA_MISMATCH");
            }
            if scope.attempt.is_some_and(|v| v != run.run_attempt || run.latest_known_attempt != run.run_attempt) {
                return api_error(StatusCode::CONFLICT, "RUN_ATTEMPT_MISMATCH");
            }
            run.target_verified = scope.head_sha.is_some() && scope.attempt == Some(run.run_attempt)
                && run.latest_known_attempt == run.run_attempt;
            response(json!({"run":run}))
        },
        Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_UNAVAILABLE"),
    }
}

fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/registry", get(registry_endpoint))
        .route("/v1/runs", get(runs_endpoint))
        .route("/v1/runs/:run_id", get(single_run_endpoint))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

fn create_or_read_token() -> Result<String> {
    let entry = Entry::new(KEYRING_SERVICE, BRIDGE_ACCOUNT)?;
    if let Ok(token) = entry.get_password() {
        if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token);
        }
        return Err(anyhow!("Invalid bridge credential; rotate it in Windows Credential Manager"));
    }
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    entry.set_password(&token)?;
    Ok(token)
}

pub(super) fn start(db_path: PathBuf) {
    let token = match create_or_read_token() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Autopilot read bridge disabled: credential setup failed: {error}");
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
            Ok(value) => value,
            Err(error) => {
                eprintln!("Autopilot read bridge unavailable: localhost bind failed: {error}");
                return;
            }
        };
        if let Err(error) = axum::serve(listener, router(Arc::new(BridgeState { db_path, token }))).await {
            eprintln!("Autopilot read bridge stopped: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_and_scope_are_fail_closed() {
        assert!(ct_token_eq("abc", "abc"));
        assert!(!ct_token_eq("abc", "abcd"));
        assert!(!ct_token_eq("abc", "abd"));
        assert!(validate_scope(&Scope::default(), true).is_err());
        let mut scope = Scope { repository_id:Some(1), ..Scope::default() };
        assert!(validate_scope(&scope,true).is_ok());
        scope.track_key=Some("INVALID!".into());
        assert!(validate_scope(&scope,true).is_err());
        scope.track_key=Some("character-memory".into());
        scope.head_sha=Some("not-a-sha".into());
        assert!(validate_scope(&scope,true).is_err());
    }

    #[test]
    fn ci_and_freshness_are_conservative() {
        assert_eq!(ci_state("queued",None),"QUEUED");
        assert_eq!(ci_state("in_progress",None),"RUNNING");
        assert_eq!(ci_state("completed",Some("success")),"SUCCESS");
        assert_eq!(ci_state("completed",Some("failure")),"FAILURE");
        assert_eq!(ci_state("completed",Some("cancelled")),"OTHER_TERMINAL");
        let now=Utc::now();
        let seen=now.to_rfc3339();
        assert_eq!(freshness(now,true,Some(&seen),None,&seen,180,true),"FRESH");
        assert_eq!(freshness(now,true,Some(&seen),Some("401"),&seen,180,true),"POLL_ERROR");
        assert_eq!(freshness(now,true,None,None,&seen,180,true),"UNKNOWN");
        assert_eq!(freshness(now,true,Some(&seen),None,&seen,180,false),"STALE");
        let past=(now-chrono::Duration::seconds(400)).to_rfc3339();
        assert_eq!(freshness(now,true,Some(&past),None,&past,180,true),"STALE");
    }

    #[tokio::test]
    async fn api_authentication_rejects_missing_credentials_and_browser_origin() {
        use tower::ServiceExt;
        let app = router(Arc::new(BridgeState {
            db_path:PathBuf::from("not-accessed-for-health.sqlite3"),
            token:"test-token".to_string(),
        }));
        let request = |authorization:Option<&str>,origin:Option<&str>| {
            let mut builder = axum::http::Request::builder()
                .method("GET").uri("/v1/health")
                .header(HOST,format!("127.0.0.1:{PORT}"));
            if let Some(auth)=authorization { builder=builder.header(AUTHORIZATION,auth); }
            if let Some(origin)=origin { builder=builder.header(ORIGIN,origin); }
            builder.body(axum::body::Body::empty()).unwrap()
        };
        let unauthenticated=app.clone().oneshot(request(None,None)).await.unwrap();
        assert_eq!(unauthenticated.status(),StatusCode::UNAUTHORIZED);
        let browser=app.clone().oneshot(request(Some("Bearer test-token"),Some("https://attacker.invalid"))).await.unwrap();
        assert_eq!(browser.status(),StatusCode::FORBIDDEN);
        let authorized=app.clone().oneshot(request(Some("Bearer test-token"),None)).await.unwrap();
        assert_eq!(authorized.status(),StatusCode::OK);
        assert!(authorized.headers().contains_key(CACHE_CONTROL));
        let unbound=app.oneshot(axum::http::Request::builder().uri("/v1/health")
            .header(HOST,"evil.example").header(AUTHORIZATION,"Bearer test-token")
            .body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(unbound.status(),StatusCode::FORBIDDEN);
    }

    #[test]
    fn bridge_reads_existing_registry_and_scoped_runs_without_mutation() {
        let suffix=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("watchtower-bridge-{}-{}.sqlite3",std::process::id(),suffix));
        init_db(&path).unwrap();
        let conn=Connection::open(&path).unwrap();
        let now=Utc::now().to_rfc3339();
        let old=(Utc::now()-chrono::Duration::hours(3)).to_rfc3339();
        let sha="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        conn.execute(
            "INSERT INTO projects(name,project_key,created_at,updated_at)
             VALUES('Test','test-project',?,?)",params![now,now],
        ).unwrap();
        let project_id=conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO monitored_repositories(project_id,repo,enabled,created_at,updated_at,last_polled_at,last_successful_poll_at)
             VALUES(?,'owner/repo',1,?,?,?,?)",params![project_id,now,now,now,now],
        ).unwrap();
        let repository_id=conn.last_insert_rowid();
        let mut track_ids=Vec::new();
        for key in ["character-memory","ops"] {
            conn.execute(
                "INSERT INTO watch_tracks(project_id,name,track_key,long_ci_minutes,active,created_at,updated_at)
                 VALUES(?,?,?,8,1,?,?)",params![project_id,key,key,now,now],
            ).unwrap();
            track_ids.push(conn.last_insert_rowid());
        }
        conn.execute(
            "INSERT INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
             VALUES(?,?,'CI',1,?)",params![project_id,repository_id,now],
        ).unwrap();
        for (run_id,status,conclusion,last_seen,resolution) in [
            (7001_i64,"completed","success",now.as_str(),"project"),
            (7002,"completed","failure",now.as_str(),"assigned"),
            (7003,"completed","success",now.as_str(),"conflict"),
            (7004,"completed","success",old.as_str(),"project"),
            (7005,"completed","success",now.as_str(),"project"),
            (7006,"queued","",now.as_str(),"project"),
        ] {
            conn.execute(
                "INSERT INTO workflow_runs(
                   run_id,repository_id,workflow_id,workflow_name,workflow_path,
                   display_title,event,head_branch,head_sha,run_number,run_attempt,status,
                   conclusion,html_url,created_at,updated_at,last_seen_at,resolution_status
                 ) VALUES(?,?,1,'CI','.github/workflows/ci.yml','CI test',
                          'pull_request','feat/bridge',?,1,1,?,?,?,?,?,?,?)",
                params![run_id,repository_id,sha,status,
                    if conclusion.is_empty(){None}else{Some(conclusion)},format!("https://example/{run_id}"),
                    now,now,last_seen,resolution],
            ).unwrap();
        }
        for (run_id,track_id) in [(7001,track_ids[0]),(7002,track_ids[1])] {
            conn.execute(
                "INSERT INTO run_track_associations(run_id,track_id,confidence,source,reason,associated_at)
                 VALUES(?,?,98,'pr_marker','PR explicit',?)",
                params![run_id,track_id,now],
            ).unwrap();
        }
        conn.execute(
            "INSERT INTO run_attempts(run_id,run_attempt,status,conclusion,updated_at)
             VALUES(7005,2,'completed','failure',?)",params![now],
        ).unwrap();

        let read=read_connection(&path).unwrap();
        assert!(read.execute("DELETE FROM watch_tracks",[]).is_err());
        let records=registry(&read,&Scope{project_id:Some(project_id),..Scope::default()}).unwrap();
        assert_eq!(records["projects"].as_array().unwrap().len(),1);
        assert_eq!(records["tracks"].as_array().unwrap().len(),2);

        let filter=Scope{
            repository_id:Some(repository_id),
            track_key:Some("character-memory".to_string()),
            head_sha:Some(sha.to_string()),
            attempt:Some(1),
            ..Scope::default()
        };
        let runs=query_runs(&read,&filter,None).unwrap();
        assert_eq!(runs.len(),1);
        assert_eq!(runs[0].run_id,7001);
        assert_eq!(runs[0].ci_state,"SUCCESS");
        assert_eq!(runs[0].evidence_state,"FRESH");
        assert_eq!(runs[0].responsibility_kind,"project-wide");
        assert_eq!(runs[0].work_track_key.as_deref(),Some("character-memory"));
        assert!(runs[0].target_verified);
        assert!(!runs[0].safe_to_advance);
        let others=query_runs(&read,&Scope{repository_id:Some(repository_id),
            track_key:Some("ops".into()),..Scope::default()},None).unwrap();
        assert_eq!(others.len(),1);
        assert_eq!(others[0].ci_state,"FAILURE");

        let all=query_runs(&read,&Scope{repository_id:Some(repository_id),..Scope::default()},None).unwrap();
        let get=|id|all.iter().find(|r|r.run_id==id).unwrap();
        assert_eq!(get(7003).decision_state,"CONFLICT");
        assert_eq!(get(7004).decision_state,"STALE");
        assert_eq!(get(7005).latest_known_attempt,2);
        assert_eq!(get(7005).decision_state,"STALE");
        assert_eq!(get(7006).ci_state,"QUEUED");
        drop(read);
        conn.execute("UPDATE monitored_repositories SET last_error='HTTP 401' WHERE id=?",
            params![repository_id]).unwrap();
        let read=read_connection(&path).unwrap();
        let all=query_runs(&read,&Scope{repository_id:Some(repository_id),..Scope::default()},None).unwrap();
        assert!(all.iter().all(|run|run.evidence_state=="POLL_ERROR"));
        assert!(all.iter().all(|run|run.poll_error_code==Some("GITHUB_AUTH_OR_PERMISSION")));
        drop(read);
        conn.execute("UPDATE monitored_repositories SET last_error=NULL WHERE id=?",
            params![repository_id]).unwrap();
        conn.execute("UPDATE watch_tracks SET active=0 WHERE id=?",
            params![track_ids[0]]).unwrap();
        let read=read_connection(&path).unwrap();
        let runs=query_runs(&read,&filter,None).unwrap();
        assert_eq!(runs.len(),1);
        assert!(!runs[0].work_track_active);
        assert_eq!(runs[0].decision_state,"UNKNOWN");
        drop(read);
        drop(conn);
        let _=std::fs::remove_file(&path);
        let _=std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _=std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[tokio::test]
    async fn api_scope_and_unavailable_database_return_contract_errors() {
        use tower::ServiceExt;
        let app = router(Arc::new(BridgeState {
            db_path: PathBuf::from("missing-bridge-test-database.sqlite3"),
            token: "test-token".into(),
        }));
        for (uri, status, code) in [
            ("/v1/runs", StatusCode::BAD_REQUEST, "REPOSITORY_SCOPE_REQUIRED"),
            ("/v1/registry?project_id=-1", StatusCode::BAD_REQUEST, "INVALID_SCOPE"),
            ("/v1/runs?repository_id=1&limit=101", StatusCode::BAD_REQUEST, "INVALID_SCOPE"),
            ("/v1/runs/0", StatusCode::BAD_REQUEST, "INVALID_RUN_ID"),
            ("/v1/runs?repository_id=1", StatusCode::SERVICE_UNAVAILABLE, "DATABASE_UNAVAILABLE"),
        ] {
            let request = axum::http::Request::builder().uri(uri)
                .header(HOST, format!("127.0.0.1:{PORT}"))
                .header(AUTHORIZATION, "Bearer test-token")
                .body(axum::body::Body::empty()).unwrap();
            let actual = app.clone().oneshot(request).await.unwrap();
            assert_eq!(actual.status(), status, "{uri}");
            let body = axum::body::to_bytes(actual.into_body(), usize::MAX).await.unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["contractVersion"], CONTRACT_VERSION);
            assert_eq!(payload["error"]["code"], code, "{uri}");
        }
    }

    #[tokio::test]
    async fn http_exact_run_rejects_different_head_and_old_attempt() {
        use tower::ServiceExt;
        let suffix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "watchtower-http-bridge-{}-{}.sqlite3", std::process::id(), suffix
        ));
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let now = Utc::now().to_rfc3339();
        let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        conn.execute("INSERT INTO projects(name,project_key,created_at,updated_at)
                      VALUES('HTTP test','http-test',?,?)", params![now, now]).unwrap();
        let project_id = conn.last_insert_rowid();
        conn.execute("INSERT INTO monitored_repositories(
                      project_id,repo,enabled,created_at,updated_at,last_polled_at,last_successful_poll_at)
                      VALUES(?,'test/http-repo',1,?,?,?,?)",
                     params![project_id, now, now, now, now]).unwrap();
        let repository_id = conn.last_insert_rowid();
        conn.execute("INSERT INTO workflow_runs(
                      run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,
                      event,head_branch,head_sha,run_number,run_attempt,status,conclusion,
                      html_url,created_at,updated_at,last_seen_at,resolution_status)
                      VALUES(12345,?,1,'CI','.github/workflows/ci.yml','CI HTTP test',
                             'pull_request','test/bridge',?,1,2,'completed','success',
                             'https://github.invalid/12345',?,?,?,'project')",
                     params![repository_id, sha, now, now, now]).unwrap();
        conn.execute("INSERT INTO run_attempts(run_id,run_attempt,status,conclusion,updated_at)
                      VALUES(12345,1,'completed','failure',?)", params![now]).unwrap();
        let app = router(Arc::new(BridgeState { db_path: path.clone(), token: "test-token".into() }));
        let cases = [
            (format!("/v1/runs/12345?repository_id={repository_id}&head_sha={sha}&attempt=2"),
             StatusCode::OK, None),
            (format!("/v1/runs/12345?repository_id={repository_id}&head_sha={sha}&attempt=1"),
             StatusCode::CONFLICT, Some("RUN_ATTEMPT_MISMATCH")),
            (format!("/v1/runs/12345?repository_id={repository_id}&head_sha={}&attempt=2",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
             StatusCode::CONFLICT, Some("HEAD_SHA_MISMATCH")),
            (format!("/v1/runs/12346?repository_id={repository_id}&head_sha={sha}&attempt=2"),
             StatusCode::NOT_FOUND, Some("RUN_NOT_FOUND")),
        ];
        for (uri, status, error) in cases {
            let request = axum::http::Request::builder().uri(uri.as_str())
                .header(HOST, format!("127.0.0.1:{PORT}"))
                .header(AUTHORIZATION, "Bearer test-token")
                .body(axum::body::Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), status, "{uri}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            if let Some(code) = error {
                assert_eq!(payload["error"]["code"], code, "{uri}");
            } else {
                assert_eq!(payload["data"]["run"]["runAttempt"], 2);
                assert_eq!(payload["data"]["run"]["targetVerified"], true);
                assert_eq!(payload["data"]["run"]["safeToAdvance"], false);
            }
        }
        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[tokio::test]
    async fn real_loopback_http_enforces_bridge_auth_and_shutdown() {
        // Unlike Router::oneshot tests, this exercises a real Windows/Linux TCP socket.
        // Dynamic port prevents interfering with a user's installed WatchTower.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        let task = tokio::spawn(async move {
            let app = router(Arc::new(BridgeState {
                db_path: PathBuf::from("no-database-required-for-health.sqlite3"),
                token: "synthetic-http-test-token".to_string(),
            }));
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build().unwrap();
        let url = format!("http://127.0.0.1:{}/v1/health", address.port());
        let host = format!("127.0.0.1:{PORT}");
        let rejected = client.get(&url).header(reqwest::header::HOST, host.as_str())
            .send().await.unwrap();
        assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(rejected.json::<Value>().await.unwrap()["error"]["code"], "UNAUTHORIZED");

        let denied_origin = client.get(&url).header(reqwest::header::HOST, host.as_str())
            .header(reqwest::header::AUTHORIZATION, "Bearer synthetic-http-test-token")
            .header(reqwest::header::ORIGIN, "https://untrusted.invalid")
            .send().await.unwrap();
        assert_eq!(denied_origin.status(), reqwest::StatusCode::FORBIDDEN);

        let denied_host = client.get(&url).header(reqwest::header::HOST, "localhost:48765")
            .header(reqwest::header::AUTHORIZATION, "Bearer synthetic-http-test-token")
            .send().await.unwrap();
        assert_eq!(denied_host.status(), reqwest::StatusCode::FORBIDDEN);

        let accepted = client.get(&url).header(reqwest::header::HOST, host)
            .header(reqwest::header::AUTHORIZATION, "Bearer synthetic-http-test-token")
            .send().await.unwrap();
        assert_eq!(accepted.status(), reqwest::StatusCode::OK);
        assert_eq!(accepted.headers().get(reqwest::header::CACHE_CONTROL).unwrap(), "no-store");
        let data = accepted.json::<Value>().await.unwrap();
        assert_eq!(data["contractVersion"], "1.0");
        assert_eq!(data["data"]["bridge"], "connected");

        task.abort();
        let _ = task.await;
        let stopped = client.get(&url).send().await;
        assert!(stopped.is_err(), "fully stopped listener must not serve stale health");
    }
}
