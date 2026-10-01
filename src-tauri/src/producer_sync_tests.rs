use super::*;
use producer_sync::{ProducerPull, ProducerSnapshot, ProducerWorkflow};

fn fixture() -> (PathBuf, Connection, MonitoredRepository) {
    let path = tests::legacy_v02_db_path("producer-discovery");
    init_db(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let now = "2026-10-01T00:00:00Z";
    let project_id = save_project_in_conn(&conn, &ProjectInput { id: None, name: "Example".into(), project_key: "example".into() }, now).unwrap();
    save_repository_in_conn(&conn, &RepositoryInput { id: None, project_id, repo: "example/producer".into(), enabled: true }, now).unwrap();
    let repo = list_repositories(&conn, false).unwrap().remove(0);
    (path, conn, repo)
}

fn snapshot(keys: &[&str]) -> ProducerSnapshot {
    ProducerSnapshot { repository: "example/producer".into(), default_branch: "main".into(), sha: "default-sha".into(),
        pulls: vec![ProducerPull { number: 12, keys: keys.iter().map(|key| key.to_string()).collect(), head_sha: "run-sha".into(), merge_sha: Some("merge-sha".into()), branch: "feat/actual-work".into() }],
        retired_keys: vec!["retired".into()], workflows: vec![ProducerWorkflow { path: ".github/workflows/ci.yml".into(), name: "CI".into(), binding: "unassigned-by-design".into(), responsibility: "project-wide-ci".into(), source: RESPONSIBILITY_MAP_PATH.into() }],
        historical_runs: Vec::new(), current_runs: Vec::new() }
}

fn run(id: i64) -> GithubRun {
    serde_json::from_value(serde_json::json!({"id":id,"workflow_id":1,"name":"CI","path":".github/workflows/ci.yml","display_title":"[WT:retired] legacy responsibility", "event":"pull_request", "head_branch":"feat/actual-work", "head_sha":"run-sha", "run_number":1,"run_attempt":1,"status":"completed","conclusion":"success","html_url":"https://github.com/example/producer/actions/runs/1","created_at":"2026-10-01T00:00:00Z","run_started_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:01:00Z"})).unwrap()
}

#[test]
fn discovery_recovers_history_without_personal_seed_or_identity_changes() {
    let (path, conn, repo) = fixture();
    upsert_run(&conn, repo.id, &run(123), "2026-10-01T00:00:00Z").unwrap();
    let mut data = snapshot(&["actual-work"]);
    data.historical_runs.push(run(124));
    assert!(producer_sync::apply(&conn, &repo, &data).unwrap());
    let tracks = list_tracks(&conn, true).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].track_key, "actual-work");
    assert_eq!(runs_for_track(&conn, tracks[0].id, 30).unwrap().len(), 2);
    let detail = load_run_attribution_detail(&conn, 123).unwrap();
    assert_eq!(detail.resolution_status, "project");
    assert_eq!(detail.work_track_key.as_deref(), Some("actual-work"));
    assert_eq!(detail.technical_responsibility.as_deref(), Some("project-wide-ci"));
    assert_eq!(list_projects(&conn, false).unwrap().len(), 1);
    assert_eq!(list_repositories(&conn, false).unwrap()[0].id, repo.id);
    assert_eq!(list_repositories(&conn, false).unwrap()[0].project_id, repo.project_id);
    assert!(list_tracks(&conn, false).unwrap().iter().all(|track| track.track_key != "retired"));
    drop(conn); let _ = std::fs::remove_file(path);
}

#[test]
fn discovery_reuses_alias_id_and_preserves_manual_history_settings_and_user_rename() {
    let (path, conn, repo) = fixture();
    let now = "2026-10-01T00:00:00Z";
    let id = save_track_in_conn(&conn, &TrackInput { id: None, project_id: repo.project_id, name: "User name".into(), track_key: "original".into(), long_ci_minutes: 19 }, now).unwrap();
    conn.execute("INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at) VALUES(?,'renamed',?,1,?)", params![repo.project_id,id,now]).unwrap();
    upsert_run(&conn, repo.id, &run(123), now).unwrap();
    upsert_run(&conn, repo.id, &run(124), now).unwrap();
    conn.execute("INSERT INTO run_assignments(run_id,track_id,confidence,source,reason,manual,assigned_at) VALUES(124,?,100,'manual','user selection',1,?)", params![id,now]).unwrap();
    conn.execute("INSERT INTO notifications_v2(track_id,run_id,run_attempt,event_type,notified_at) VALUES(?,124,1,'success',?)",params![id,now]).unwrap();
    producer_sync::apply(&conn, &repo, &snapshot(&["renamed"])).unwrap();
    let tracks = list_tracks(&conn, false).unwrap();
    assert_eq!(tracks.len(),1); assert_eq!(tracks[0].id,id); assert_eq!(tracks[0].name,"User name"); assert_eq!(tracks[0].long_ci_minutes,19);
    assert_eq!(load_run_attribution_detail(&conn,124).unwrap().manual,Some(true));
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM notifications_v2",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    conn.execute("UPDATE watch_tracks SET name='After sync',track_key='user-new-key',active=0 WHERE id=?",params![id]).unwrap();
    assert!(!producer_sync::apply(&conn,&repo,&snapshot(&["renamed","new-key"])).unwrap());
    let track = list_tracks(&conn,false).unwrap().remove(0);
    assert_eq!(track.name,"After sync"); assert_eq!(track.track_key,"user-new-key"); assert!(!track.active);
    assert_eq!(load_settings(&conn).unwrap().active_poll_seconds,DEFAULT_ACTIVE_POLL_SECONDS);
    drop(conn); let _ = std::fs::remove_file(path);
}

#[test]
fn discovery_retirement_is_inactive_and_invalid_or_conflicting_explicit_keys_fail_closed() {
    for keys in [vec!["bad&key"],vec!["one","two"],vec!["retired"]] {
        let (path,conn,repo)=fixture(); let now="2026-10-01T00:00:00Z";
        let id=save_track_in_conn(&conn,&TrackInput{id:None,project_id:repo.project_id,name:"Retired".into(),track_key:"retired".into(),long_ci_minutes:8},now).unwrap();
        upsert_run(&conn,repo.id,&run(123),now).unwrap();
        producer_sync::apply(&conn,&repo,&snapshot(&keys)).unwrap();
        let tracks=list_tracks(&conn,false).unwrap(); assert_eq!(tracks.len(),1); assert_eq!(tracks[0].id,id); assert!(!tracks[0].active);
        assert!(load_run_attribution_detail(&conn,123).unwrap().work_track_key.is_none());
        assert!(conn.query_row("SELECT COUNT(*) FROM run_track_associations",[],|row|row.get::<_,i64>(0)).unwrap()==0);
        drop(conn); let _=std::fs::remove_file(path);
    }
    let tracks=vec![tests::track(1,"one"),tests::track(2,"two")];
    let evidence=producer_sync::trailer_keys("Watchtower-Track: one\nWatchtower-Track: two").into_iter().map(|key| tests::evidence(&key,"pr_marker",98)).collect();
    assert_eq!(resolve_evidence(&tracks,&HashMap::new(),evidence).status,"conflict");
}

#[test]
fn producer_markers_and_rules_are_repository_scoped_and_pr_work_is_independent() {
    let data=snapshot(&["actual-work"]);
    let mut evidence=vec![tests::evidence("retired","run_name",100),tests::evidence("actual-work","pr_marker",98)];
    producer_sync::filter_evidence(&data,Some(".github/workflows/ci.yml"),"CI",&mut evidence);
    assert_eq!(evidence.len(),1);
    let tracks=vec![tests::track(1,"technical"),tests::track(2,"actual-work")];
    let (association,_)=work_track_association(&tracks,&HashMap::new(),&[tests::evidence("technical","run_name",100),tests::evidence("actual-work","pr_marker",98)]);
    assert_eq!(association.unwrap().0,2);
    let (path,conn,repo)=fixture(); producer_sync::apply(&conn,&repo,&data).unwrap();
    assert!(!project_rule_matches(&list_project_workflow_rules(&conn).unwrap(),repo.project_id,repo.id+1,"CI"));
    let mut other=repo.clone(); other.id+=1; other.repo="example/other".into();
    assert!(producer_sync::apply(&conn,&other,&data).is_err());
    drop(conn); let _=std::fs::remove_file(path);
}

#[test]
fn current_responsibility_formats_and_absent_map_do_not_discover_work_tracks() {
    let (responsibility,binding)=producer_sync::entry_binding(&serde_json::json!({"responsibility":"project-wide-ci","workTrackAttribution":"pr-metadata"}),"CI");
    assert_eq!(responsibility,"project-wide-ci"); assert_eq!(binding,"unassigned-by-design");
    assert_eq!(producer_sync::entry_binding(&serde_json::json!({"responsibility":"commerce-verification","workTrackAttribution":"none"}),"DB Commerce").1,"dynamic-by-run");
    assert_eq!(producer_sync::entry_binding(&serde_json::json!({"primaryResponsibility":"catalog-taxonomy","watchtowerTrackBinding":"static:taxonomy-ai"}),"Taxonomy").1,"static:taxonomy-ai");
    assert_eq!(producer_sync::entry_binding(&serde_json::Value::Null,"CI").1,"unassigned-by-design");
    assert_eq!(producer_sync::entry_binding(&serde_json::Value::Null,"Database Checks").1,"dynamic-by-run");
}

#[test]
fn restart_does_not_resurrect_suppressed_legacy_marker_and_keeps_original_evidence() {
    let (path, conn, repo) = fixture();
    let now = "2026-10-01T00:00:00Z";
    let id = save_track_in_conn(&conn, &TrackInput { id: None, project_id: repo.project_id, name: "Technical legacy".into(), track_key: "technical".into(), long_ci_minutes: 8 }, now).unwrap();
    upsert_run(&conn,repo.id,&run(123),now).unwrap();
    conn.execute("INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) VALUES(123,'technical','run_name',100,'legacy',?)",params![now]).unwrap();
    persist_work_track_association(&conn,123,Some((id,100,"run_name".into(),"legacy".into())),true,now).unwrap();
    let mut data=snapshot(&[]); data.pulls.clear();
    producer_sync::apply(&conn,&repo,&data).unwrap();
    assert!(load_run_attribution_detail(&conn,123).unwrap().work_track_key.is_none());
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM run_evidence WHERE run_id=123 AND value='legacy'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(conn); init_db(&path).unwrap(); let conn=Connection::open(&path).unwrap();
    assert!(load_run_attribution_detail(&conn,123).unwrap().work_track_key.is_none());
    drop(conn); let _=std::fs::remove_file(path);
}

#[test]
fn active_run_outside_recent_window_requires_exact_github_reconciliation() {
    let (path,conn,repo)=fixture();
    let mut old=run(123); old.status="queued".into(); old.conclusion=None;
    upsert_run(&conn,repo.id,&old,"2026-10-01T00:00:00Z").unwrap();
    assert_eq!(producer_sync::missing_active_run_ids(&conn,repo.id,&[run(124)]).unwrap(),vec![123]);
    assert!(producer_sync::missing_active_run_ids(&conn,repo.id+1,&[]).unwrap().is_empty());
    assert!(producer_sync::missing_active_run_ids(&conn,repo.id,&[old]).unwrap().is_empty());
    upsert_run(&conn,repo.id,&run(123),"2026-10-01T00:01:00Z").unwrap();
    assert!(producer_sync::missing_active_run_ids(&conn,repo.id,&[]).unwrap().is_empty());
    drop(conn);let _=std::fs::remove_file(path);
}
