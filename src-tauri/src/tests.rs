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

fn init_visualy_test_fixture(path: &Path) {
    init_db(path).unwrap();
    legacy_compat::seed_bejewely_project_scope(&Connection::open(path).unwrap()).unwrap();
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
fn generic_alias_tokens_are_preserved_until_project_scoped_resolution() {
    assert_eq!(
        extract_marker("[WT:legacy&ops] Shared Validation"),
        Some("legacy&ops".into())
    );
    assert_eq!(
        extract_track_trailer("feat: x\n\nWatchtower-Track: legacy&ops"),
        Some("legacy&ops".into())
    );
    assert!(branch_has_key("feat/legacy&ops/provider-quality", "legacy&ops"));

    let tracks = vec![track(1, "ops")];
    let aliases = HashMap::from([("legacy&ops".into(), "ops".into())]);
    let resolution = resolve_evidence(
        &tracks,
        &aliases,
        vec![evidence("legacy&ops", "run_name", 100)],
    );
    assert_eq!(resolution.status, "assigned");
    assert_eq!(resolution.track_id, Some(1));
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
    assert_eq!(legacy_compat::legacy_track_key("프론트 연동 4", 1), "frontend-integration");
    assert_eq!(legacy_compat::legacy_track_key("운영 32", 2), "ops");
    assert_eq!(legacy_compat::legacy_track_key("관상 연구 및 검증 2", 3), "face-research");
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
               workflow_path TEXT,
               resolution_status TEXT NOT NULL,
               ignored INTEGER NOT NULL
             );
             CREATE TABLE run_assignments(
               run_id INTEGER PRIMARY KEY,
               track_id INTEGER NOT NULL
             );
             INSERT INTO monitored_repositories(id,project_id)
               VALUES(10,1),(20,1),(30,2);
             INSERT INTO workflow_runs(run_id,repository_id,workflow_path,resolution_status,ignored)
               VALUES(101,10,NULL,'unassigned',0),
                     (102,10,NULL,'conflict',0),
                     (103,10,NULL,'project',0),
                     (104,10,NULL,'unassigned',1),
                     (201,20,NULL,'unassigned',0),
                     (202,20,NULL,'project',0),
                     (301,30,NULL,'project',0),
                     (302,30,NULL,'unassigned',0);
             INSERT INTO run_assignments(run_id,track_id) VALUES(102,999);",
    )
    .unwrap();

    let stats = repository_scope_stats(&conn).unwrap();
    let by_repo: HashMap<i64, (i64, i64, i64)> = stats
        .into_iter()
        .map(|item| {
            (
                item.repository_id,
                (
                    item.project_id,
                    item.unassigned_count,
                    item.project_run_count,
                ),
            )
        })
        .collect();

    assert_eq!(by_repo.get(&10), Some(&(1, 1, 1)));
    assert_eq!(by_repo.get(&20), Some(&(1, 1, 1)));
    assert_eq!(by_repo.get(&30), Some(&(2, 1, 1)));
}

#[test]
fn dependabot_dynamic_workflows_do_not_pollute_attribution_surfaces() {
    let path = legacy_v02_db_path("dependabot-dynamic-noise");
    init_visualy_test_fixture(&path);

    let conn = Connection::open(&path).unwrap();
    let repository_id: i64 = conn
        .query_row(
            "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/K_beauty'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    for (run_id, workflow_id, workflow_name, workflow_path, event, created_at) in [
        (
            9_400_001_i64,
            940_i64,
            "Undeclared Repository Workflow",
            ".github/workflows/undeclared.yml",
            "pull_request",
            "2026-09-24T04:00:00Z",
        ),
        (
            9_400_002_i64,
            941_i64,
            "npm_and_yarn in /. - Update #1589675854",
            "dynamic/dependabot/dependabot-updates",
            "dynamic",
            "2026-09-24T04:01:00Z",
        ),
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
                    workflow_id,
                    workflow_name,
                    workflow_path,
                    workflow_name,
                    event,
                    "main",
                    format!("sha-{run_id}"),
                    run_id,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    created_at,
                    Option::<String>::None,
                    created_at,
                    created_at,
                    "unassigned",
                    0_i64,
                    Option::<String>::None,
                ],
            )
            .unwrap();
    }

    let inbox = unassigned_runs_for_repository(&conn, repository_id, 50).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].id, 9_400_001_i64);

    let contract_runs = producer_contract_runs(&conn, 50).unwrap();
    let repo_contract_runs: Vec<_> = contract_runs
        .iter()
        .filter(|item| item.run.repository_id == repository_id)
        .collect();
    assert_eq!(repo_contract_runs.len(), 1);
    assert_eq!(repo_contract_runs[0].run.id, 9_400_001_i64);
    assert!(repo_contract_runs[0].is_current_producer_run);

    let stats = producer_contract_stats(&conn, 50).unwrap();
    let repo_stats = stats
        .iter()
        .find(|item| item.repository_id == repository_id)
        .unwrap();
    assert_eq!(repo_stats.sampled_runs, 1);
    assert_eq!(repo_stats.unresolved_runs, 1);

    let scope = repository_scope_stats(&conn).unwrap();
    let repo_scope = scope
        .iter()
        .find(|item| item.repository_id == repository_id)
        .unwrap();
    assert_eq!(repo_scope.unassigned_count, 1);

    let unresolved = load_stored_unresolved_runs(&conn, repository_id, 50).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].id, 9_400_001_i64);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
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
               VALUES(500,'unassigned',NULL);",
    )
    .unwrap();

    let resolution = Resolution {
        status: "unassigned".into(),
        track_id: None,
        confidence: Some(40),
        source: None,
        reason: Some("insufficient".into()),
        evidence: vec![evidence("ops", "workflow_name", 40)],
    };
    persist_resolution(&conn, 500, &resolution, "2026-09-24T01:00:00Z").unwrap();

    let row: (String, Option<String>) = conn
        .query_row(
            "SELECT resolution_status,last_resolution_attempt_at
             FROM workflow_runs WHERE run_id=500",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
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
               VALUES(500,'ops','workflow_name',50,'CI','2026-09-24T00:00:00Z');",
    )
    .unwrap();

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
    )
    .unwrap();

    let row: (String, String, Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT trigger,from_status,to_track_key,to_source,evidence_json
             FROM resolution_reconciliation_audit
             WHERE run_id=500",
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
             INSERT INTO workflow_runs VALUES(501,100,'unassigned',NULL);",
    )
    .unwrap();

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
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM resolution_reconciliation_audit WHERE run_id=501",
            [],
            |row| row.get(0),
        )
        .unwrap();
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
             );",
    )
    .unwrap();

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
        )
        .unwrap();
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
             INSERT INTO run_assignments(run_id,manual) VALUES(99,1);",
    )
    .unwrap();

    let first = load_stored_unresolved_runs(&conn, 100, 12).unwrap();
    assert_eq!(first.len(), 12);
    assert!(!first.iter().any(|run| run.id == 99));

    for run in &first {
        conn.execute(
            "UPDATE workflow_runs
                 SET last_resolution_attempt_at='2026-09-24T01:00:00Z'
                 WHERE run_id=?",
            params![run.id],
        )
        .unwrap();
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
             ) VALUES(500,10,100,'manual','사용자 수동 귀속',1,'2026-09-23T00:00:00Z');",
    )
    .unwrap();

    legacy_compat::invalidate_cross_project_assignments(&conn, 100, Some(1), 2, "visualy-project-scope-v1")
        .unwrap();

    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);

    let archived: (i64, String, String, i64, i64) = conn
        .query_row(
            "SELECT manual,track_key,reason,from_repository_project_id,to_repository_project_id
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
    assert_eq!(archived.0, 1);
    assert_eq!(archived.1, "old-track");
    assert_eq!(archived.2, "사용자 수동 귀속");
    assert_eq!(archived.3, 1);
    assert_eq!(archived.4, 2);

    let status: String = conn
        .query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=500",
            [],
            |row| row.get(0),
        )
        .unwrap();
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
                     (600,20,100,'manual','valid visualy manual',1,'2026-09-23T00:00:00Z');",
    )
    .unwrap();

    legacy_compat::invalidate_cross_project_assignments(&conn, 100, Some(1), 2, "visualy-project-scope-v1")
        .unwrap();
    legacy_compat::invalidate_cross_project_assignments(&conn, 100, Some(1), 2, "visualy-project-scope-v1")
        .unwrap();

    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assignment_migration_audit",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 1);

    let valid_manual_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments
             WHERE run_id=600 AND track_id=20 AND manual=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(valid_manual_count, 1);

    let invalid_auto_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        )
        .unwrap();
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
    for (run_id, repository_id) in [
        (9_100_006_i64, myeongha_repository_id),
        (9_100_007_i64, saju_repository_id),
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
                    912_i64,
                    "Web Auth Browser Regression",
                    ".github/workflows/web-browser-auth-regression.yml",
                    "web auth browser regression",
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

    legacy_compat::migrate_project_scope_legacy(&Connection::open(&path).unwrap()).unwrap();

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
                     'Web Browser Smoke',
                     'Web Auth Browser Regression'
                   )
                   AND active=1",
            params![project_id, myeongha_repository_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scoped_rules, 6);

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

    let web_auth_status: String = conn
        .query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=9100006",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let web_auth_assignment_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=9100006",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(web_auth_status, "project");
    assert_eq!(web_auth_assignment_count, 0);

    let saju_web_auth_row: (String, i64) = conn
        .query_row(
            "SELECT wr.resolution_status,COUNT(ra.run_id)
                 FROM workflow_runs wr
                 LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
                 WHERE wr.run_id=9100007
                 GROUP BY wr.run_id,wr.resolution_status",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saju_web_auth_row, ("assigned".into(), 1));

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn myeongha_records_production_smoke_is_dynamic_without_forcing_track_assignment() {
    let path = legacy_v02_db_path("myeongha-records-dynamic");
    seed_legacy_v02_database(&path);
    init_db(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let repository_id: i64 = conn
        .query_row(
            "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/MyeongHa'",
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

    let dynamic_rule: (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),MAX(protected) FROM dynamic_workflow_rules
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='Production Records Current-Subject Smoke'
                   AND active=1",
            params![project_id, repository_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(dynamic_rule, (1, 1));

    let project_rule_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM project_workflow_rules
                 WHERE project_id=? AND workflow_name='Production Records Current-Subject Smoke'
                   AND active=1",
            params![project_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(project_rule_count, 0);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn visualy_repository_responsibility_map_uses_dynamic_rules_and_retires_stale_project_wide_security(
) {
    let path = legacy_v02_db_path("visualy-responsibility-map");
    seed_legacy_v02_database(&path);
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

    let project_wide_names: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT workflow_name FROM project_workflow_rules
                     WHERE project_id=? AND active=1
                     ORDER BY workflow_name",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![project_id], |row| row.get(0))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    assert_eq!(
        project_wide_names,
        vec![
            "BEJEWELY Current Main Health".to_string(),
            "PIE Prospective Shadow".to_string(),
        ]
    );

    let dynamic_names: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT workflow_name,protected FROM dynamic_workflow_rules
                     WHERE project_id=? AND repository_id=? AND active=1
                     ORDER BY workflow_name",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![project_id, repository_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    assert_eq!(dynamic_names.len(), 12);
    assert!(dynamic_names.iter().all(|(_, protected)| *protected == 1));
    for expected in [
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
        assert!(dynamic_names.iter().any(|(name, _)| name == expected));
    }

    conn.execute(
        "INSERT INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,NULL,'BEJEWELY Security Boundary',1,'2026-09-24T00:00:00Z')",
        params![project_id],
    )
    .unwrap();
    conn.execute(
            "INSERT INTO workflow_runs(
               run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
               head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
               created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                9_200_001_i64,
                repository_id,
                920_i64,
                "BEJEWELY Security Boundary",
                ".github/workflows/security-boundary.yml",
                "BEJEWELY Security Boundary",
                "push",
                "main",
                "sha-9200001",
                9200001_i64,
                1_i64,
                "completed",
                "success",
                "https://example/9200001",
                "2026-09-24T01:00:00Z",
                Option::<String>::None,
                "2026-09-24T01:01:00Z",
                "2026-09-24T01:01:00Z",
                "project",
                0_i64,
                Option::<String>::None,
            ],
        )
        .unwrap();
    drop(conn);

    legacy_compat::seed_bejewely_project_scope(&Connection::open(&path).unwrap()).unwrap();

    let conn = Connection::open(&path).unwrap();
    let stale_rule_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_workflow_rules
                 WHERE project_id=?
                   AND workflow_name IN ('BEJEWELY Security Boundary','BEJEWELY Supply Chain Security')",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(stale_rule_count, 0);

    let corrected_status: String = conn
        .query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=9200001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(corrected_status, "unassigned");

    let mesh_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dynamic_workflow_rules
                 WHERE project_id=? AND repository_id=? AND active=1",
            params![project_id, repository_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mesh_count, 12);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn dynamic_workflow_rule_declares_responsibility_without_forcing_track_ownership() {
    let path = legacy_v02_db_path("dynamic-workflow-responsibility");
    seed_legacy_v02_database(&path);
    init_db(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let repository_id: i64 = conn
        .query_row(
            "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/Saju'",
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
            "SELECT id FROM watch_tracks WHERE project_id=? AND active=1 ORDER BY id LIMIT 1",
            params![project_id],
            |row| row.get(0),
        )
        .unwrap();

    let dynamic_rule: (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),MAX(protected) FROM dynamic_workflow_rules
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='MESH6J Manual Browser Capture Surface CI' AND active=1",
            params![project_id, repository_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(dynamic_rule, (1, 1));

    let listed_rules = list_dynamic_workflow_rules(&conn).unwrap();
    let mesh_rule = listed_rules
        .iter()
        .find(|rule| {
            rule.project_id == project_id
                && rule.repository_id == Some(repository_id)
                && rule.workflow_name == "MESH6J Manual Browser Capture Surface CI"
        })
        .unwrap();
    assert!(mesh_rule.active);
    assert!(mesh_rule.protected);

    conn.execute(
            "INSERT INTO workflow_runs(
               run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,
               head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,
               created_at,run_started_at,updated_at,last_seen_at,resolution_status,ignored,last_resolution_attempt_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                9_220_001_i64,
                repository_id,
                922_i64,
                "MESH6J Manual Browser Capture Surface CI",
                ".github/workflows/mesh6j-localhost-manual-browser-capture-ci.yml",
                "research(face-reading): FR274",
                "pull_request",
                "research/face-research/fr274-still-image-diagnostic",
                "sha-9220001",
                9220001_i64,
                1_i64,
                "completed",
                "success",
                "https://example/9220001",
                "2026-09-24T03:00:00Z",
                Option::<String>::None,
                "2026-09-24T03:01:00Z",
                "2026-09-24T03:01:00Z",
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
            9_220_001_i64,
            track_id,
            98_i64,
            "pr_marker",
            "PR #1407",
            "2026-09-24T03:02:00Z",
        ],
    )
    .unwrap();

    let rows = producer_contract_runs(&conn, 50).unwrap();
    let mesh = rows
        .iter()
        .find(|item| item.run.id == 9_220_001_i64)
        .unwrap();
    assert!(mesh.contract_compliant);
    assert!(mesh.responsibility_declared);
    assert_eq!(mesh.bucket, "pr_marker");
    assert_eq!(mesh.run.resolution_status, "assigned");

    conn.execute(
        "UPDATE workflow_runs
             SET workflow_name='New Shared Gate',workflow_id=923
             WHERE run_id=9220001",
        [],
    )
    .unwrap();
    let rows = producer_contract_runs(&conn, 50).unwrap();
    let undeclared = rows
        .iter()
        .find(|item| item.run.id == 9_220_001_i64)
        .unwrap();
    assert!(undeclared.contract_compliant);
    assert!(!undeclared.responsibility_declared);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn producer_contract_stats_classify_recent_runs_without_overlapping_buckets() {
    let path = legacy_v02_db_path("producer-contract");
    init_visualy_test_fixture(&path);

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
        repository_runs
            .iter()
            .filter(|item| item.contract_compliant)
            .count(),
        4
    );
    assert!(repository_runs
        .iter()
        .all(|item| item.is_current_producer_run));
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
fn producer_contract_runs_separate_current_from_historical_drift_by_workflow_identity() {
    let path = legacy_v02_db_path("producer-contract-current");
    init_visualy_test_fixture(&path);

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

    for (run_id, workflow_id, workflow_name, created_at, resolution_status) in [
        (
            9_300_001_i64,
            8_301_i64,
            "Recovered Producer",
            "2026-09-24T00:01:00Z",
            "unassigned",
        ),
        (
            9_300_002_i64,
            8_301_i64,
            "Recovered Producer",
            "2026-09-24T00:02:00Z",
            "assigned",
        ),
        (
            9_300_003_i64,
            8_302_i64,
            "Still Drifting Producer",
            "2026-09-24T00:03:00Z",
            "unassigned",
        ),
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
                    workflow_id,
                    workflow_name,
                    Option::<String>::None,
                    workflow_name,
                    "push",
                    "main",
                    format!("sha-{run_id}"),
                    run_id,
                    1_i64,
                    "completed",
                    "success",
                    format!("https://example/{run_id}"),
                    created_at,
                    Option::<String>::None,
                    created_at,
                    created_at,
                    resolution_status,
                    0_i64,
                    Option::<String>::None,
                ],
            )
            .unwrap();
    }

    conn.execute(
        "INSERT INTO run_assignments(
               run_id,track_id,confidence,source,reason,manual,assigned_at
             ) VALUES(?,?,?,?,?,?,?)",
        params![
            9_300_002_i64,
            track_id,
            100_i64,
            "run_name",
            "fixture",
            0_i64,
            "2026-09-24T00:02:30Z",
        ],
    )
    .unwrap();

    let contract_runs = producer_contract_runs(&conn, 50).unwrap();
    let older = contract_runs
        .iter()
        .find(|item| item.run.id == 9_300_001)
        .unwrap();
    let recovered = contract_runs
        .iter()
        .find(|item| item.run.id == 9_300_002)
        .unwrap();
    let active_drift = contract_runs
        .iter()
        .find(|item| item.run.id == 9_300_003)
        .unwrap();

    assert!(!older.contract_compliant);
    assert!(!older.is_current_producer_run);

    assert!(recovered.contract_compliant);
    assert!(recovered.is_current_producer_run);

    assert!(!active_drift.contract_compliant);
    assert!(active_drift.is_current_producer_run);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn fresh_database_starts_without_project_specific_seed_data() {
    let path = legacy_v02_db_path("fresh-core-decoupled");
    init_db(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    for table in [
        "projects",
        "monitored_repositories",
        "watch_tracks",
        "project_workflow_rules",
        "dynamic_workflow_rules",
        "track_aliases",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} should be empty on a fresh install");
    }

    let migration_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key='core-decoupling-v032'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migration_count, 1);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn generic_registry_survives_restart_without_project_seed_reinjection() {
    let path = legacy_v02_db_path("generic-registry-restart");
    init_db(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let now = "2026-09-25T00:00:00Z";
    conn.execute(
        "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES('Example Product','example-product',1,?,?)",
        params![now, now],
    )
    .unwrap();
    let first_project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES('Second Product','second-product',1,?,?)",
        params![now, now],
    )
    .unwrap();
    let second_project_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO monitored_repositories(project_id,repo,enabled,created_at,updated_at) VALUES(?,'example/service',1,?,?)",
        params![first_project_id, now, now],
    )
    .unwrap();
    let repository_id = conn.last_insert_rowid();

    for project_id in [first_project_id, second_project_id] {
        conn.execute(
            "INSERT INTO watch_tracks(project_id,name,track_key,long_ci_minutes,active,created_at,updated_at) VALUES(?,'Operations','ops',8,1,?,?)",
            params![project_id, now, now],
        )
        .unwrap();
    }

    conn.execute(
        "INSERT INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at) VALUES(?,?,'CI',1,?)",
        params![first_project_id, repository_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO dynamic_workflow_rules(project_id,repository_id,workflow_name,active,protected,created_at) VALUES(?,?,'Shared Validation',1,0,?)",
        params![first_project_id, repository_id, now],
    )
    .unwrap();
    drop(conn);

    init_db(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let projects: Vec<String> = {
        let mut stmt = conn.prepare("SELECT project_key FROM projects ORDER BY project_key").unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(projects, vec!["example-product".to_string(), "second-product".to_string()]);

    let ops_tracks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM watch_tracks WHERE track_key='ops'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ops_tracks, 2);

    let repository_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM monitored_repositories", [], |row| row.get(0))
        .unwrap();
    let project_rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM project_workflow_rules", [], |row| row.get(0))
        .unwrap();
    let dynamic_rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM dynamic_workflow_rules", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repository_count, 1);
    assert_eq!(project_rule_count, 1);
    assert_eq!(dynamic_rule_count, 1);

    let forbidden_projects: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE project_key IN ('visualy','myeongha','default')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(forbidden_projects, 0);

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
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
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
        .query_row("SELECT COUNT(*) FROM track_fingerprints", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(fingerprint_count, 2);

    let notification_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM notifications_v2", [], |row| {
            row.get(0)
        })
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

    let migrated_project_id: i64 = conn
        .query_row(
            "SELECT project_id FROM track_aliases WHERE alias_key='privacy-recovery'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrated_project_id, 1);

    conn.execute(
        "INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at)
             VALUES(2,'privacy-recovery',20,1,'now')",
        [],
    )
    .unwrap();

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

    let manual: i64 = conn
        .query_row(
            "SELECT manual FROM run_assignments WHERE run_id=500",
            [],
            |row| row.get(0),
        )
        .unwrap();
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
fn responsibility_map_source_health_preserves_last_good_snapshot_on_failure() {
    let path = legacy_v02_db_path("responsibility-map-source-health");
    init_visualy_test_fixture(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute("PRAGMA foreign_keys=ON", []).unwrap();

    let repository_id: i64 = conn
        .query_row(
            "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/K_beauty'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let contracts = vec![RepositoryResponsibilityContract {
        workflow_path: ".github/workflows/current-main-health.yml".into(),
        workflow_name: "BEJEWELY Current Main Health".into(),
        binding_kind: "project-wide".into(),
        track_key: None,
        source_binding: "unassigned-by-design".into(),
        source_path: RESPONSIBILITY_MAP_PATH.into(),
    }];
    replace_repository_responsibility_contracts(
        &conn,
        repository_id,
        &contracts,
        "2026-09-24T07:00:00Z",
    )
    .unwrap();
    update_repository_responsibility_source(
        &conn,
        repository_id,
        "synced",
        "2026-09-24T07:00:00Z",
        None,
    )
    .unwrap();

    update_repository_responsibility_source(
        &conn,
        repository_id,
        "error",
        "2026-09-24T07:05:00Z",
        Some("HTTP 503"),
    )
    .unwrap();

    let statuses = responsibility_map_source_statuses(&conn).unwrap();
    let visualy = statuses
        .iter()
        .find(|item| item.repository_id == repository_id)
        .unwrap();
    assert_eq!(visualy.status, "error");
    assert_eq!(
        visualy.last_attempt_at.as_deref(),
        Some("2026-09-24T07:05:00Z")
    );
    assert_eq!(
        visualy.last_success_at.as_deref(),
        Some("2026-09-24T07:00:00Z")
    );
    assert_eq!(visualy.last_error.as_deref(), Some("HTTP 503"));
    assert_eq!(visualy.contract_count, 1);

    let contract_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM repository_responsibility_contracts WHERE repository_id=?",
            params![repository_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(contract_count, 1);

    update_repository_responsibility_source(
        &conn,
        repository_id,
        "not_found",
        "2026-09-24T07:10:00Z",
        None,
    )
    .unwrap();
    let statuses = responsibility_map_source_statuses(&conn).unwrap();
    let visualy = statuses
        .iter()
        .find(|item| item.repository_id == repository_id)
        .unwrap();
    assert_eq!(visualy.status, "not_found");
    assert_eq!(
        visualy.last_success_at.as_deref(),
        Some("2026-09-24T07:00:00Z")
    );
    assert_eq!(visualy.contract_count, 1);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn responsibility_map_drift_is_detect_only_and_repository_scoped() {
    let path = legacy_v02_db_path("responsibility-map-drift");
    init_visualy_test_fixture(&path);
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

    let now = "2026-09-24T06:30:00Z";
    conn.execute(
        "INSERT OR IGNORE INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,?,'Kind Mismatch',1,?)",
        params![project_id, repository_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,'Kind Mismatch Project',1,1,?)",
        params![project_id, repository_id, now],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO monitored_repositories(
               project_id,repo,enabled,running_count,queued_count,created_at,updated_at
             ) VALUES(?,'example/other-visualy-repo',1,0,0,?,?)",
        params![project_id, now, now],
    )
    .unwrap();
    let other_repository_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,'Scoped Dynamic',1,0,?)",
        params![project_id, other_repository_id, now],
    )
    .unwrap();

    for (run_id, workflow_id, workflow_name, workflow_path, display_title) in [
        (
            9_800_001_i64,
            9801_i64,
            "Stale Dynamic",
            ".github/workflows/stale-dynamic.yml",
            "Stale Dynamic",
        ),
        (
            9_800_002_i64,
            9802_i64,
            "Stale Project",
            ".github/workflows/stale-project.yml",
            "Stale Project",
        ),
        (
            9_800_003_i64,
            9803_i64,
            "Static Clean",
            ".github/workflows/static-clean.yml",
            "[WT:mobile] Static Clean",
        ),
        (
            9_800_004_i64,
            9804_i64,
            "Wrong Static",
            ".github/workflows/wrong-static.yml",
            "[WT:trust] Wrong Static",
        ),
        (
            9_800_005_i64,
            9805_i64,
            "Missing Project",
            ".github/workflows/missing-project.yml",
            "Missing Project",
        ),
    ] {
        let run = GithubRun {
            id: run_id,
            workflow_id,
            name: workflow_name.into(),
            path: Some(workflow_path.into()),
            display_title: Some(display_title.into()),
            event: "push".into(),
            head_branch: Some("main".into()),
            head_sha: format!("sha-{run_id}"),
            run_number: run_id,
            run_attempt: 1,
            status: "completed".into(),
            conclusion: Some("success".into()),
            html_url: format!("https://example/{run_id}"),
            created_at: now.into(),
            run_started_at: Some(now.into()),
            updated_at: now.into(),
            pull_requests: vec![],
        };
        upsert_run(&conn, repository_id, &run, now).unwrap();
    }

    conn.execute(
        "INSERT OR IGNORE INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,'Stale Dynamic',1,1,?)",
        params![project_id, repository_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,?,'Stale Project',1,?)",
        params![project_id, repository_id, now],
    )
    .unwrap();

    let contracts = vec![
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/current-main-health.yml".into(),
            workflow_name: "BEJEWELY Current Main Health".into(),
            binding_kind: "project-wide".into(),
            track_key: None,
            source_binding: "unassigned-by-design".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/security-boundary.yml".into(),
            workflow_name: "BEJEWELY Security Boundary".into(),
            binding_kind: "dynamic".into(),
            track_key: None,
            source_binding: "dynamic-by-run".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/static-clean.yml".into(),
            workflow_name: "Static Clean".into(),
            binding_kind: "static".into(),
            track_key: Some("mobile".into()),
            source_binding: "static:mobile".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/missing-dynamic.yml".into(),
            workflow_name: "Missing Dynamic".into(),
            binding_kind: "dynamic".into(),
            track_key: None,
            source_binding: "dynamic-by-run".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/missing-project.yml".into(),
            workflow_name: "Missing Project".into(),
            binding_kind: "project-wide".into(),
            track_key: None,
            source_binding: "unassigned-by-design".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/kind-mismatch.yml".into(),
            workflow_name: "Kind Mismatch".into(),
            binding_kind: "dynamic".into(),
            track_key: None,
            source_binding: "dynamic-by-run".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/kind-mismatch-project.yml".into(),
            workflow_name: "Kind Mismatch Project".into(),
            binding_kind: "project-wide".into(),
            track_key: None,
            source_binding: "unassigned-by-design".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/wrong-static.yml".into(),
            workflow_name: "Wrong Static".into(),
            binding_kind: "static".into(),
            track_key: Some("mobile".into()),
            source_binding: "static:mobile".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/scoped-dynamic.yml".into(),
            workflow_name: "Scoped Dynamic".into(),
            binding_kind: "dynamic".into(),
            track_key: None,
            source_binding: "dynamic-by-run".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
        RepositoryResponsibilityContract {
            workflow_path: ".github/workflows/ghost-static.yml".into(),
            workflow_name: "Ghost Static".into(),
            binding_kind: "static".into(),
            track_key: Some("ghost-track".into()),
            source_binding: "static:ghost-track".into(),
            source_path: RESPONSIBILITY_MAP_PATH.into(),
        },
    ];
    replace_repository_responsibility_contracts(&conn, repository_id, &contracts, now).unwrap();

    let track_count_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM watch_tracks", [], |row| row.get(0))
        .unwrap();

    let mobile_track_id: i64 = conn
        .query_row(
            "SELECT id FROM watch_tracks WHERE project_id=? AND track_key='mobile' AND active=1",
            params![project_id],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO run_assignments(
               run_id,track_id,confidence,source,reason,manual,assigned_at
             ) VALUES(9_800_005,?,100,'manual','manual preservation fixture',1,?)",
        params![mobile_track_id, now],
    )
    .unwrap();
    let assignment_count_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM run_assignments", [], |row| row.get(0))
        .unwrap();

    let drifts = responsibility_map_drifts(&conn).unwrap();

    let track_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM watch_tracks", [], |row| row.get(0))
        .unwrap();
    let assignment_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM run_assignments", [], |row| row.get(0))
        .unwrap();
    assert_eq!(track_count_before, track_count_after);
    assert_eq!(assignment_count_before, assignment_count_after);

    assert!(!drifts
        .iter()
        .any(|item| item.workflow_name == "BEJEWELY Current Main Health"));
    assert!(!drifts
        .iter()
        .any(|item| item.workflow_name == "BEJEWELY Security Boundary"));
    assert!(!drifts
        .iter()
        .any(|item| item.workflow_name == "Static Clean"));

    let missing_names: HashSet<&str> = drifts
        .iter()
        .filter(|item| item.drift_type == "missing_in_watchtower")
        .map(|item| item.workflow_name.as_str())
        .collect();
    assert!(missing_names.contains("Missing Dynamic"));
    assert!(missing_names.contains("Missing Project"));
    assert!(missing_names.contains("Scoped Dynamic"));

    let missing_dynamic = drifts
        .iter()
        .find(|item| item.workflow_name == "Missing Dynamic")
        .unwrap();
    assert_eq!(missing_dynamic.source_path, RESPONSIBILITY_MAP_PATH);
    assert_eq!(
        missing_dynamic.recommended_action,
        "review_add_dynamic_rule"
    );
    assert!(missing_dynamic
        .reason
        .contains("no matching active Dynamic rule"));

    let kind_mismatch = drifts
        .iter()
        .find(|item| item.workflow_name == "Kind Mismatch")
        .unwrap();
    assert_eq!(kind_mismatch.drift_type, "responsibility_kind_mismatch");
    assert_eq!(
        kind_mismatch.recommended_action,
        "review_reclassify_dynamic"
    );
    assert!(kind_mismatch.reason.contains("dynamic-by-run"));
    assert!(kind_mismatch.reason.contains("project-wide"));
    let wrong_static = drifts
        .iter()
        .find(|item| item.workflow_name == "Wrong Static")
        .unwrap();
    assert_eq!(wrong_static.drift_type, "track_binding_mismatch");
    assert_eq!(wrong_static.expected_track_key.as_deref(), Some("mobile"));
    assert_eq!(wrong_static.actual_track_key.as_deref(), Some("trust"));
    assert_eq!(wrong_static.recommended_action, "review_producer_run_name");
    assert!(wrong_static.reason.contains("static:mobile"));
    assert!(wrong_static.reason.contains("trust"));

    let ghost_static = drifts
        .iter()
        .find(|item| item.workflow_name == "Ghost Static")
        .unwrap();
    assert_eq!(ghost_static.drift_type, "track_binding_mismatch");
    assert_eq!(
        ghost_static.recommended_action,
        "review_track_registry_or_map"
    );
    assert!(ghost_static.reason.contains("canonical Track"));

    let stale_dynamic = drifts
        .iter()
        .find(|item| item.workflow_name == "Stale Dynamic")
        .unwrap();
    assert_eq!(stale_dynamic.drift_type, "stale_in_watchtower");
    assert_eq!(stale_dynamic.watchtower_binding.as_deref(), Some("dynamic"));
    assert_eq!(
        stale_dynamic.recommended_action,
        "review_remove_or_confirm_stale_rule"
    );

    let stale_project = drifts
        .iter()
        .find(|item| item.workflow_name == "Stale Project")
        .unwrap();
    assert_eq!(stale_project.drift_type, "stale_in_watchtower");
    assert_eq!(
        stale_project.watchtower_binding.as_deref(),
        Some("project-wide")
    );
    assert_eq!(
        stale_project.recommended_action,
        "review_remove_or_confirm_stale_rule"
    );

    let preview = resolution_preview_for_drift(&conn, missing_dynamic).unwrap();
    assert!(preview.executable);
    assert_eq!(preview.action, "add_dynamic_rule");
    assert!(preview
        .changes
        .iter()
        .any(|item| item.contains("protected Dynamic")));
    assert!(
        exact_dynamic_rule(&conn, project_id, repository_id, "Missing Dynamic")
            .unwrap()
            .is_none()
    );

    let deferred = defer_responsibility_drift_with_conn(
        &conn,
        &DeferResponsibilityDriftInput {
            review_key: missing_dynamic.review_key.clone(),
            fingerprint: missing_dynamic.fingerprint.clone(),
        },
    )
    .unwrap();
    assert_eq!(deferred.status, "deferred");
    let deferred_drift = current_responsibility_drift(&conn, &missing_dynamic.review_key).unwrap();
    assert_eq!(deferred_drift.review_status, "deferred");
    let reopened = reopen_responsibility_drift_with_conn(
        &conn,
        &DeferResponsibilityDriftInput {
            review_key: missing_dynamic.review_key.clone(),
            fingerprint: missing_dynamic.fingerprint.clone(),
        },
    )
    .unwrap();
    assert_eq!(reopened.status, "open");
    let reopened_drift = current_responsibility_drift(&conn, &missing_dynamic.review_key).unwrap();
    assert_eq!(reopened_drift.review_status, "open");

    let resolved_missing_dynamic = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: missing_dynamic.review_key.clone(),
            fingerprint: missing_dynamic.fingerprint.clone(),
            action: "add_dynamic_rule".into(),
        },
    )
    .unwrap();
    assert_eq!(resolved_missing_dynamic.status, "resolved");
    let created_dynamic = exact_dynamic_rule(&conn, project_id, repository_id, "Missing Dynamic")
        .unwrap()
        .unwrap();
    assert!(created_dynamic.protected);

    let missing_project = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Missing Project")
        .unwrap();
    let resolved_missing_project = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: missing_project.review_key.clone(),
            fingerprint: missing_project.fingerprint.clone(),
            action: "add_project_wide_rule".into(),
        },
    )
    .unwrap();
    assert_eq!(resolved_missing_project.status, "resolved");
    assert!(
        exact_project_rule(&conn, project_id, repository_id, "Missing Project")
            .unwrap()
            .is_some()
    );
    let manual_preserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=9_800_005 AND manual=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(manual_preserved, 1);

    let kind_dynamic = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Kind Mismatch")
        .unwrap();
    let kind_dynamic_result = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: kind_dynamic.review_key.clone(),
            fingerprint: kind_dynamic.fingerprint.clone(),
            action: "reclassify_to_dynamic".into(),
        },
    )
    .unwrap();
    assert_eq!(kind_dynamic_result.status, "resolved");
    assert!(
        exact_project_rule(&conn, project_id, repository_id, "Kind Mismatch")
            .unwrap()
            .is_none()
    );
    assert!(
        exact_dynamic_rule(&conn, project_id, repository_id, "Kind Mismatch")
            .unwrap()
            .unwrap()
            .protected
    );

    let kind_project = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Kind Mismatch Project")
        .unwrap();
    let kind_project_result = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: kind_project.review_key.clone(),
            fingerprint: kind_project.fingerprint.clone(),
            action: "reclassify_to_project_wide".into(),
        },
    )
    .unwrap();
    assert_eq!(kind_project_result.status, "resolved");
    assert!(
        exact_dynamic_rule(&conn, project_id, repository_id, "Kind Mismatch Project")
            .unwrap()
            .is_none()
    );
    assert!(
        exact_project_rule(&conn, project_id, repository_id, "Kind Mismatch Project")
            .unwrap()
            .is_some()
    );

    let stale_dynamic_current = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Stale Dynamic")
        .unwrap();
    let stale_dynamic_result = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: stale_dynamic_current.review_key.clone(),
            fingerprint: stale_dynamic_current.fingerprint.clone(),
            action: "remove_stale_rule".into(),
        },
    )
    .unwrap();
    assert_eq!(stale_dynamic_result.status, "resolved");
    assert!(
        exact_dynamic_rule(&conn, project_id, repository_id, "Stale Dynamic")
            .unwrap()
            .is_none()
    );

    let wrong_static_current = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Wrong Static")
        .unwrap();
    let blocked_preview = resolution_preview_for_drift(&conn, &wrong_static_current).unwrap();
    assert!(!blocked_preview.executable);
    assert_eq!(blocked_preview.action, "blocked");
    let blocked_result = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: wrong_static_current.review_key.clone(),
            fingerprint: wrong_static_current.fingerprint.clone(),
            action: "blocked".into(),
        },
    )
    .unwrap();
    assert_eq!(blocked_result.status, "blocked");

    let scoped_dynamic = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Scoped Dynamic")
        .unwrap();
    let old_fingerprint = scoped_dynamic.fingerprint.clone();
    conn.execute(
        "UPDATE repository_responsibility_contracts
             SET binding_kind='project-wide',source_binding='unassigned-by-design'
             WHERE repository_id=? AND workflow_name='Scoped Dynamic'",
        params![repository_id],
    )
    .unwrap();
    let stale_rejected = resolve_responsibility_drift_with_conn(
        &conn,
        &ResolveResponsibilityDriftInput {
            review_key: scoped_dynamic.review_key.clone(),
            fingerprint: old_fingerprint,
            action: "add_dynamic_rule".into(),
        },
    )
    .unwrap();
    assert_eq!(stale_rejected.status, "stale_rejected");
    assert!(
        exact_dynamic_rule(&conn, project_id, repository_id, "Scoped Dynamic")
            .unwrap()
            .is_none()
    );
    let stale_attention = current_responsibility_drift(&conn, &scoped_dynamic.review_key).unwrap();
    assert_eq!(stale_attention.review_status, "attention");
    let reopened_stale = reopen_responsibility_drift_with_conn(
        &conn,
        &DeferResponsibilityDriftInput {
            review_key: stale_attention.review_key.clone(),
            fingerprint: stale_attention.fingerprint.clone(),
        },
    )
    .unwrap();
    assert_eq!(reopened_stale.status, "open");
    assert_eq!(
        current_responsibility_drift(&conn, &stale_attention.review_key)
            .unwrap()
            .review_status,
        "open"
    );

    let failed_drift = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Stale Project")
        .unwrap();
    let failed_binding = current_watchtower_responsibility_binding(&conn, &failed_drift).unwrap();
    insert_resolution_audit(
        &conn,
        &failed_drift,
        "remove_stale_rule",
        &failed_drift.fingerprint,
        &failed_drift.fingerprint,
        failed_binding.as_deref(),
        "failed",
    )
    .unwrap();
    assert_eq!(
        current_responsibility_drift(&conn, &failed_drift.review_key)
            .unwrap()
            .review_status,
        "attention"
    );
    let reopened_failed = reopen_responsibility_drift_with_conn(
        &conn,
        &DeferResponsibilityDriftInput {
            review_key: failed_drift.review_key.clone(),
            fingerprint: failed_drift.fingerprint.clone(),
        },
    )
    .unwrap();
    assert_eq!(reopened_failed.status, "open");

    let track_count_final: i64 = conn
        .query_row("SELECT COUNT(*) FROM watch_tracks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(track_count_before, track_count_final);
    let audit_results: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT result FROM responsibility_resolution_audit ORDER BY id")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(audit_results.contains(&"deferred".to_string()));
    assert!(audit_results.contains(&"resolved".to_string()));
    assert!(audit_results.contains(&"blocked".to_string()));
    assert!(audit_results.contains(&"stale_rejected".to_string()));

    let history = responsibility_resolution_history(&conn, 200).unwrap();
    let deferred_history = history
        .iter()
        .find(|item| item.id == deferred.audit_id)
        .unwrap();
    assert_eq!(deferred_history.result, "deferred");
    assert_eq!(deferred_history.before_watchtower_contract, None);
    assert_eq!(deferred_history.after_watchtower_contract, None);
    assert!(!deferred_history.stale);

    let missing_dynamic_history: Vec<&ResponsibilityResolutionAuditEntry> = history
        .iter()
        .filter(|item| item.review_key == missing_dynamic.review_key)
        .collect();
    assert_eq!(missing_dynamic_history.len(), 3);
    assert!(missing_dynamic_history
        .iter()
        .any(|item| item.result == "deferred"));
    assert!(missing_dynamic_history
        .iter()
        .any(|item| item.action == "reopen" && item.result == "still_open"));
    let resolved_dynamic_history = missing_dynamic_history
        .iter()
        .find(|item| item.result == "resolved")
        .unwrap();
    assert_eq!(
        resolved_dynamic_history
            .after_watchtower_contract
            .as_deref(),
        Some("dynamic")
    );
    assert_eq!(
        resolved_dynamic_history.repository_contract,
        "dynamic-by-run"
    );

    let resolved_project_history = history
        .iter()
        .find(|item| item.id == resolved_missing_project.audit_id)
        .unwrap();
    assert_eq!(
        resolved_project_history
            .after_watchtower_contract
            .as_deref(),
        Some("project-wide")
    );
    let reclassified_dynamic_history = history
        .iter()
        .find(|item| item.id == kind_dynamic_result.audit_id)
        .unwrap();
    assert_eq!(
        reclassified_dynamic_history
            .before_watchtower_contract
            .as_deref(),
        Some("project-wide")
    );
    assert_eq!(
        reclassified_dynamic_history
            .after_watchtower_contract
            .as_deref(),
        Some("dynamic")
    );
    let reclassified_project_history = history
        .iter()
        .find(|item| item.id == kind_project_result.audit_id)
        .unwrap();
    assert_eq!(
        reclassified_project_history
            .before_watchtower_contract
            .as_deref(),
        Some("dynamic")
    );
    assert_eq!(
        reclassified_project_history
            .after_watchtower_contract
            .as_deref(),
        Some("project-wide")
    );
    let stale_history = history
        .iter()
        .find(|item| item.id == stale_rejected.audit_id)
        .unwrap();
    assert_eq!(stale_history.result, "stale_rejected");
    assert!(stale_history.stale);
    assert_ne!(
        stale_history.requested_fingerprint,
        stale_history.current_fingerprint
    );
    let blocked_history = history
        .iter()
        .find(|item| item.id == blocked_result.audit_id)
        .unwrap();
    assert_eq!(blocked_history.result, "blocked");
    assert_eq!(blocked_history.repository_id, repository_id);

    let deferred_row_unchanged: String = conn
        .query_row(
            "SELECT result FROM responsibility_resolution_audit WHERE id=?",
            params![deferred.audit_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(deferred_row_unchanged, "deferred");
    let manual_assignment_final: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=9_800_005 AND manual=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(manual_assignment_final, 1);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn repository_responsibility_binding_normalizes_supported_contracts() {
    assert_eq!(
        normalize_repository_binding("unassigned-by-design"),
        ("project-wide".into(), None)
    );
    assert_eq!(
        normalize_repository_binding("dynamic-by-run"),
        ("dynamic".into(), None)
    );
    assert_eq!(
        normalize_repository_binding("static:ops"),
        ("static".into(), Some("ops".into()))
    );
    assert_eq!(
        normalize_repository_binding("static:legacy&ops"),
        ("unknown".into(), None)
    );
    assert_eq!(
        normalize_repository_binding("future-contract"),
        ("unknown".into(), None)
    );
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

    let manual: (i64, i64, String) = conn
        .query_row(
            "SELECT track_id,manual,source FROM run_assignments WHERE run_id=1000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
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
    let automatic: (i64, i64, String) = conn
        .query_row(
            "SELECT track_id,manual,source FROM run_assignments WHERE run_id=1001",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(automatic, (10, 0, "inference".into()));

    assign_run_to_project_in_conn(&conn, 1001, 1, true, "2026-09-24T01:12:00Z").unwrap();

    let rules = list_project_workflow_rules(&conn).unwrap();
    assert!(project_rule_matches(&rules, 1, 100, "Shared CI"));
    assert!(project_rule_matches(&rules, 1, 110, "Shared CI"));
    assert!(!project_rule_matches(&rules, 2, 200, "Shared CI"));

    let promoted_status: String = conn
        .query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=1001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(promoted_status, "project");
    let promoted_assignment_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_assignments WHERE run_id=1001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(promoted_assignment_count, 0);

    let preserved_manual: (String, i64) = conn
        .query_row(
            "SELECT wr.resolution_status,ra.manual
             FROM workflow_runs wr
             JOIN run_assignments ra ON ra.run_id=wr.run_id
             WHERE wr.run_id=1000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(preserved_manual, ("assigned".into(), 1));

    let other_project_status: String = conn
        .query_row(
            "SELECT resolution_status FROM workflow_runs WHERE run_id=2000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(other_project_status, "unassigned");
}

#[test]
fn explicit_conflict_model_has_distinct_keys() {
    let tracks = vec![track(1, "ops"), track(2, "saju")];
    let known: HashSet<_> = tracks.iter().map(|t| t.track_key.as_str()).collect();
    assert!(known.contains("ops"));
    assert!(known.contains("saju"));
}

#[test]
fn responsibility_review_priority_and_age_bucket_follow_operational_order() {
    let policy = default_responsibility_review_policy(1);
    assert_eq!(
        responsibility_review_priority(&policy, "attention", 0),
        "p0"
    );
    assert_eq!(responsibility_review_priority(&policy, "open", 72), "p1");
    assert_eq!(responsibility_review_priority(&policy, "open", 71), "p2");
    assert_eq!(
        responsibility_review_priority(&policy, "deferred", 120),
        "p3"
    );
    assert_eq!(
        responsibility_review_priority(&policy, "blocked", 120),
        "blocked"
    );
    assert_eq!(responsibility_review_age_bucket(23), "fresh");
    assert_eq!(responsibility_review_age_bucket(24), "aging");
    assert_eq!(responsibility_review_age_bucket(71), "aging");
    assert_eq!(responsibility_review_age_bucket(72), "overdue");
    assert!(responsibility_review_priority_rank("p0") < responsibility_review_priority_rank("p1"));
    assert!(responsibility_review_priority_rank("p1") < responsibility_review_priority_rank("p2"));
    assert!(responsibility_review_priority_rank("p2") < responsibility_review_priority_rank("p3"));
}

#[test]
fn responsibility_review_sla_escalates_p0_and_p1_without_escalating_p2() {
    let policy = default_responsibility_review_policy(1);
    let p0_fresh = responsibility_review_sla(&policy, "p0", 0);
    assert_eq!(p0_fresh.0, "within_sla");
    assert_eq!(p0_fresh.1, Some(24));
    assert_eq!(p0_fresh.2, Some(24));
    assert_eq!(p0_fresh.3, "warning");
    let p0_due = responsibility_review_sla(&policy, "p0", 12);
    assert_eq!(p0_due.0, "due_soon");
    assert_eq!(p0_due.2, Some(12));
    let p0_breached = responsibility_review_sla(&policy, "p0", 24);
    assert_eq!(p0_breached.0, "breached");
    assert_eq!(p0_breached.3, "critical");
    let p1_due = responsibility_review_sla(&policy, "p1", 72);
    assert_eq!(p1_due.0, "due_soon");
    assert_eq!(p1_due.1, Some(96));
    assert_eq!(p1_due.2, Some(24));
    assert_eq!(p1_due.3, "warning");
    let p1_breached = responsibility_review_sla(&policy, "p1", 100);
    assert_eq!(p1_breached.0, "breached");
    assert_eq!(p1_breached.2, Some(-4));
    assert_eq!(p1_breached.3, "critical");
    let p2_due = responsibility_review_sla(&policy, "p2", 48);
    assert_eq!(p2_due.0, "due_soon");
    assert_eq!(p2_due.3, "none");
    let deferred = responsibility_review_sla(&policy, "p3", 500);
    assert_eq!(deferred.0, "exempt");
    assert_eq!(deferred.1, None);
    assert!(responsibility_escalation_rank("critical") < responsibility_escalation_rank("warning"));
    assert!(responsibility_escalation_rank("warning") < responsibility_escalation_rank("none"));
}

#[test]
fn responsibility_escalation_delivery_is_deduplicated_and_retry_bounded() {
    let path = legacy_v02_db_path("responsibility-escalation-delivery");
    init_visualy_test_fixture(&path);
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
    let drift = ResponsibilityMapDrift {
        project_id,
        repository_id,
        repository: "gycha0109-beep/K_beauty".into(),
        workflow_path: ".github/workflows/test.yml".into(),
        workflow_name: "Test Responsibility".into(),
        drift_type: "missing_in_watchtower".into(),
        repository_binding: "dynamic".into(),
        watchtower_binding: None,
        expected_track_key: None,
        actual_track_key: None,
        source_path: RESPONSIBILITY_MAP_PATH.into(),
        reason: "test".into(),
        recommended_action: "review_add_dynamic_rule".into(),
        review_key: "review-key".into(),
        fingerprint: "fingerprint".into(),
        review_status: "attention".into(),
        review_priority: "p0".into(),
        review_age_bucket: "fresh".into(),
        review_age_hours: 0,
        review_event_count: 0,
        failed_attempt_count: 0,
        last_reviewed_at: None,
        first_seen_at: "2026-09-25T00:00:00Z".into(),
        sla_status: "within_sla".into(),
        sla_target_hours: Some(24),
        sla_remaining_hours: Some(24),
        escalation_level: "warning".into(),
        escalation_reason: Some("test warning".into()),
        operator_state: "active".into(),
        operator_actor: None,
        operator_updated_at: None,
        operator_suppressed_until: None,
    };
    assert_eq!(
        responsibility_escalation_event_type("warning"),
        Some("warning")
    );
    assert_eq!(
        responsibility_escalation_event_type("critical"),
        Some("critical")
    );
    assert_eq!(responsibility_escalation_event_type("none"), None);

    assert!(reserve_responsibility_escalation_delivery(
        &conn,
        &drift,
        "warning",
        "2026-09-25T00:00:00Z"
    )
    .unwrap());
    finish_responsibility_escalation_delivery(
        &conn,
        &drift.review_key,
        &drift.fingerprint,
        "warning",
        "2026-09-25T00:00:00Z",
        Ok(()),
    )
    .unwrap();
    assert!(!reserve_responsibility_escalation_delivery(
        &conn,
        &drift,
        "warning",
        "2026-09-25T00:01:00Z"
    )
    .unwrap());

    for minute in 2..=4 {
        assert!(reserve_responsibility_escalation_delivery(
            &conn,
            &drift,
            "critical",
            &format!("2026-09-25T00:0{minute}:00Z"),
        )
        .unwrap());
        finish_responsibility_escalation_delivery(
            &conn,
            &drift.review_key,
            &drift.fingerprint,
            "critical",
            &format!("2026-09-25T00:0{minute}:00Z"),
            Err("delivery failed".into()),
        )
        .unwrap();
    }
    assert!(!reserve_responsibility_escalation_delivery(
        &conn,
        &drift,
        "critical",
        "2026-09-25T00:05:00Z"
    )
    .unwrap());

    let deliveries = responsibility_escalation_deliveries(&conn, 10).unwrap();
    assert_eq!(deliveries.len(), 2);
    let warning = deliveries
        .iter()
        .find(|item| item.event_type == "warning")
        .unwrap();
    assert_eq!(warning.status, "emitted");
    assert_eq!(warning.attempts, 1);
    let critical = deliveries
        .iter()
        .find(|item| item.event_type == "critical")
        .unwrap();
    assert_eq!(critical.status, "failed");
    assert_eq!(critical.attempts, 3);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn responsibility_escalation_operator_lifecycle_is_audited_and_fingerprint_scoped() {
    let path = legacy_v02_db_path("responsibility-escalation-operator-lifecycle");
    init_visualy_test_fixture(&path);
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
    conn.execute(
        "INSERT INTO repository_responsibility_contracts(
           repository_id,workflow_path,workflow_name,binding_kind,track_key,
           source_binding,source_path,last_seen_at
         ) VALUES(?,?,?,?,?,?,?,?)",
        params![
            repository_id,
            ".github/workflows/operator-test.yml",
            "Operator Lifecycle Test",
            "dynamic",
            Option::<String>::None,
            "dynamic-by-run",
            RESPONSIBILITY_MAP_PATH,
            Utc::now().to_rfc3339(),
        ],
    )
    .unwrap();

    let initial = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Operator Lifecycle Test")
        .unwrap();
    assert_eq!(initial.operator_state, "active");

    let aged = (Utc::now() - chrono::Duration::hours(80)).to_rfc3339();
    conn.execute(
        "UPDATE responsibility_review_state SET first_seen_at=?
         WHERE review_key=? AND fingerprint=?",
        params![aged, initial.review_key, initial.fingerprint],
    )
    .unwrap();
    let escalated = current_responsibility_drift(&conn, &initial.review_key).unwrap();
    assert_eq!(escalated.review_priority, "p1");
    assert_eq!(escalated.escalation_level, "warning");

    let input = ResponsibilityEscalationOperatorInput {
        review_key: escalated.review_key.clone(),
        fingerprint: escalated.fingerprint.clone(),
        suppress_hours: None,
    };
    let acknowledged = set_responsibility_escalation_operator_state_with_conn(
        &conn,
        &input,
        "acknowledged",
        "acknowledge",
    )
    .unwrap();
    assert_eq!(acknowledged.status, "acknowledged");
    assert!(acknowledged.audit_id.is_some());

    let suppressed = set_responsibility_escalation_operator_state_with_conn(
        &conn,
        &input,
        "suppressed",
        "suppress",
    )
    .unwrap();
    assert_eq!(suppressed.operator_state, "suppressed");

    let active = set_responsibility_escalation_operator_state_with_conn(
        &conn,
        &input,
        "active",
        "activate",
    )
    .unwrap();
    assert_eq!(active.operator_state, "active");

    let suppressed_again = set_responsibility_escalation_operator_state_with_conn(
        &conn,
        &input,
        "suppressed",
        "suppress",
    )
    .unwrap();
    assert_eq!(suppressed_again.operator_state, "suppressed");

    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM responsibility_escalation_operator_audit
             WHERE review_key=? AND fingerprint=?",
            params![input.review_key, input.fingerprint],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 4);

    conn.execute(
        "UPDATE repository_responsibility_contracts
         SET binding_kind='project-wide',source_binding='unassigned-by-design'
         WHERE repository_id=? AND workflow_name='Operator Lifecycle Test'",
        params![repository_id],
    )
    .unwrap();
    let changed = current_responsibility_drift(&conn, &input.review_key).unwrap();
    assert_ne!(changed.fingerprint, input.fingerprint);
    assert_eq!(changed.operator_state, "active");
    assert!(changed.operator_actor.is_none());
    let previous_state: String = conn
        .query_row(
            "SELECT state FROM responsibility_escalation_operator_state
             WHERE review_key=? AND fingerprint=?",
            params![input.review_key, input.fingerprint],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(previous_state, "suppressed");

    let stale = set_responsibility_escalation_operator_state_with_conn(
        &conn,
        &input,
        "acknowledged",
        "acknowledge",
    )
    .unwrap();
    assert_eq!(stale.status, "stale_rejected");
    assert_eq!(stale.operator_state, "active");
    let audit_count_after_stale: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM responsibility_escalation_operator_audit
             WHERE review_key=? AND fingerprint=?",
            params![input.review_key, input.fingerprint],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count_after_stale, 4);

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn responsibility_timed_suppression_expires_and_can_be_resnoozed() {
    let path = legacy_v02_db_path("responsibility-timed-suppression");
    init_visualy_test_fixture(&path);
    let conn = Connection::open(&path).unwrap();
    let repository_id: i64 = conn
        .query_row(
            "SELECT id FROM monitored_repositories WHERE repo='gycha0109-beep/K_beauty'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO repository_responsibility_contracts(
           repository_id,workflow_path,workflow_name,binding_kind,track_key,
           source_binding,source_path,last_seen_at
         ) VALUES(?,?,?,?,?,?,?,?)",
        params![
            repository_id,
            ".github/workflows/timed-operator-test.yml",
            "Timed Operator Lifecycle Test",
            "dynamic",
            Option::<String>::None,
            "dynamic-by-run",
            RESPONSIBILITY_MAP_PATH,
            Utc::now().to_rfc3339(),
        ],
    )
    .unwrap();

    let initial = responsibility_map_drifts(&conn)
        .unwrap()
        .into_iter()
        .find(|item| item.workflow_name == "Timed Operator Lifecycle Test")
        .unwrap();
    let aged = (Utc::now() - chrono::Duration::hours(80)).to_rfc3339();
    conn.execute(
        "UPDATE responsibility_review_state SET first_seen_at=?
         WHERE review_key=? AND fingerprint=?",
        params![aged, initial.review_key, initial.fingerprint],
    )
    .unwrap();
    let escalated = current_responsibility_drift(&conn, &initial.review_key).unwrap();
    assert_eq!(escalated.escalation_level, "warning");

    let input = ResponsibilityEscalationOperatorInput {
        review_key: escalated.review_key.clone(),
        fingerprint: escalated.fingerprint.clone(),
        suppress_hours: Some(4),
    };
    let first_until = (Utc::now() + chrono::Duration::hours(4)).to_rfc3339();
    let suppressed = set_responsibility_escalation_operator_state_with_conn_until(
        &conn,
        &input,
        "suppressed",
        "suppress",
        Some(&first_until),
    )
    .unwrap();
    assert_eq!(suppressed.operator_state, "suppressed");
    assert_eq!(
        suppressed.current_drift.unwrap().operator_suppressed_until,
        Some(first_until.clone())
    );

    let second_until = (Utc::now() + chrono::Duration::hours(24)).to_rfc3339();
    let resnoozed = set_responsibility_escalation_operator_state_with_conn_until(
        &conn,
        &input,
        "suppressed",
        "suppress",
        Some(&second_until),
    )
    .unwrap();
    assert_eq!(resnoozed.operator_state, "suppressed");
    assert_eq!(
        resnoozed.current_drift.unwrap().operator_suppressed_until,
        Some(second_until.clone())
    );

    let past_until = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    set_responsibility_escalation_operator_state_with_conn_until(
        &conn,
        &input,
        "suppressed",
        "suppress",
        Some(&past_until),
    )
    .unwrap();
    let expired = current_responsibility_drift(&conn, &input.review_key).unwrap();
    assert_eq!(expired.operator_state, "active");
    assert_eq!(expired.operator_actor.as_deref(), Some("system-expiry"));
    assert!(expired.operator_suppressed_until.is_none());

    let (action, actor, before_state, after_state, before_until, after_until): (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT action,actor,before_state,after_state,
                    before_suppressed_until,after_suppressed_until
             FROM responsibility_escalation_operator_audit
             WHERE review_key=? AND fingerprint=?
             ORDER BY id DESC LIMIT 1",
            params![input.review_key, input.fingerprint],
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
        .unwrap();
    assert_eq!(action, "activate");
    assert_eq!(actor, "system-expiry");
    assert_eq!(before_state, "suppressed");
    assert_eq!(after_state, "active");
    assert_eq!(before_until, Some(past_until));
    assert!(after_until.is_none());

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn responsibility_review_policy_is_project_scoped_and_preserves_defaults() {
    let path = legacy_v02_db_path("responsibility-review-policy");
    init_visualy_test_fixture(&path);
    let conn = Connection::open(&path).unwrap();
    let visualy_project_id: i64 = conn
        .query_row(
            "SELECT id FROM projects WHERE project_key='visualy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO projects(name,project_key,active,created_at,updated_at)
         VALUES('Policy Isolation','policy-isolation',1,'now','now')",
        [],
    )
    .unwrap();
    let isolation_project_id = conn.last_insert_rowid();

    let visualy_default = responsibility_review_policy(&conn, visualy_project_id).unwrap();
    assert_eq!(visualy_default.p0_target_hours, 24);
    assert_eq!(visualy_default.p1_target_hours, 96);
    assert_eq!(visualy_default.p2_target_hours, 72);
    assert!(visualy_default.notify_warning);
    assert!(visualy_default.updated_at.is_none());

    conn.execute(
        "INSERT INTO responsibility_review_policies(
           project_id,p0_target_hours,p1_target_hours,p2_target_hours,
           p0_due_soon_hours,p1_due_soon_hours,p2_due_soon_hours,
           notify_warning,notify_critical,updated_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?)",
        params![
            visualy_project_id,
            12,
            120,
            48,
            6,
            12,
            12,
            0,
            1,
            "2026-09-25T01:00:00Z"
        ],
    )
    .unwrap();

    let visualy = responsibility_review_policy(&conn, visualy_project_id).unwrap();
    let isolated = responsibility_review_policy(&conn, isolation_project_id).unwrap();
    assert_eq!(visualy.p0_target_hours, 12);
    assert_eq!(visualy.p1_target_hours, 120);
    assert_eq!(visualy.p2_target_hours, 48);
    assert!(!visualy.notify_warning);
    assert!(visualy.notify_critical);
    assert_eq!(isolated.p0_target_hours, 24);
    assert_eq!(isolated.p2_target_hours, 72);
    assert!(isolated.updated_at.is_none());

    assert_eq!(responsibility_review_priority(&visualy, "open", 47), "p2");
    assert_eq!(responsibility_review_priority(&visualy, "open", 48), "p1");
    let p0 = responsibility_review_sla(&visualy, "p0", 6);
    assert_eq!(p0.0, "due_soon");
    assert_eq!(p0.1, Some(12));
    assert_eq!(p0.2, Some(6));

    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}
