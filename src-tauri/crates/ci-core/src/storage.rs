//! Shared Run/Attempt persistence for WatchTower and future headless Core.
//! The caller owns the SQLite connection, repository identity and single-writer process lease.
//! No schema creation, polling, PAT, process management or automatic approval is performed.
use anyhow::Result;
use rusqlite::{params,Connection};
use crate::github::GithubRun;

pub fn upsert_run(conn: &Connection, repository_id: i64, run: &GithubRun, now: &str) -> Result<()> {
    // A SAVEPOINT also works inside a transaction started by the Tauri producer.
    // Each Run + Attempt pair is atomic without attempting a nested BEGIN.
    conn.execute_batch("SAVEPOINT ci_core_run_attempt")?;
    match upsert_run_inner(conn,repository_id,run,now) {
        Ok(())=>{
            conn.execute_batch("RELEASE SAVEPOINT ci_core_run_attempt")?;
            Ok(())
        },
        Err(original)=>{
            conn.execute_batch("ROLLBACK TO SAVEPOINT ci_core_run_attempt; RELEASE SAVEPOINT ci_core_run_attempt")?;
            Err(original)
        }
    }
}
fn upsert_run_inner(conn: &Connection, repository_id: i64, run: &GithubRun, now: &str) -> Result<()> {
    let title = run
        .display_title
        .clone()
        .unwrap_or_else(|| run.name.clone());
    let mut pr_numbers: Vec<i64> = run.pull_requests.iter().map(|pr| pr.number).filter(|id| *id > 0).collect();
    pr_numbers.sort_unstable();
    pr_numbers.dedup();
    let pr_numbers_json = serde_json::to_string(&pr_numbers)?;
    conn.execute(
        "INSERT INTO workflow_runs(run_id,repository_id,workflow_id,workflow_name,workflow_path,display_title,event,head_branch,head_sha,run_number,run_attempt,status,conclusion,html_url,created_at,run_started_at,updated_at,last_seen_at,pr_numbers_json)
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
         ON CONFLICT(run_id) DO UPDATE SET
           repository_id=excluded.repository_id,workflow_id=excluded.workflow_id,workflow_name=excluded.workflow_name,
           workflow_path=excluded.workflow_path,display_title=excluded.display_title,event=excluded.event,
           head_branch=excluded.head_branch,head_sha=excluded.head_sha,run_number=excluded.run_number,
           run_attempt=excluded.run_attempt,status=excluded.status,conclusion=excluded.conclusion,
           html_url=excluded.html_url,run_started_at=excluded.run_started_at,updated_at=excluded.updated_at,last_seen_at=excluded.last_seen_at,
           pr_numbers_json=CASE WHEN excluded.pr_numbers_json!='[]' THEN excluded.pr_numbers_json ELSE workflow_runs.pr_numbers_json END",
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
            now,
            pr_numbers_json
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


#[cfg(test)]
mod tests{
    use super::*;
    fn schema(conn:&Connection){
        conn.execute_batch("
            CREATE TABLE workflow_runs(
                run_id INTEGER PRIMARY KEY,repository_id INTEGER,workflow_id INTEGER,
                workflow_name TEXT,workflow_path TEXT,display_title TEXT,event TEXT,
                head_branch TEXT,head_sha TEXT,run_number INTEGER,run_attempt INTEGER,
                status TEXT,conclusion TEXT,html_url TEXT,created_at TEXT,run_started_at TEXT,
                updated_at TEXT,last_seen_at TEXT,pr_numbers_json TEXT NOT NULL,
                resolution_status TEXT NOT NULL DEFAULT 'unassigned');
            CREATE TABLE run_attempts(
                run_id INTEGER,run_attempt INTEGER,status TEXT,conclusion TEXT,
                started_at TEXT,updated_at TEXT,PRIMARY KEY(run_id,run_attempt));
            CREATE TABLE run_assignments(
                run_id INTEGER PRIMARY KEY,track_id INTEGER,source TEXT,manual INTEGER);
        ").unwrap();
    }
    fn run(attempt:i64,status:&str,conclusion:Option<&str>,with_pr:bool)->GithubRun{
        serde_json::from_value(serde_json::json!({
            "id":551,"workflow_id":8,"name":"CI","path":".github/workflows/ci.yml",
            "display_title":"CI: PR #9","event":"pull_request",
            "head_branch":"topic","head_sha":"a".repeat(40),"run_number":7,
            "run_attempt":attempt,"status":status,"conclusion":conclusion,
            "html_url":"https://github.com/a/b/actions/runs/551",
            "created_at":"2026-11-01T00:00:00Z",
            "run_started_at":null,"updated_at":"2026-11-01T00:02:00Z",
            "pull_requests":if with_pr {serde_json::json!([{"number":9},{"number":9}])}
                else {serde_json::json!([])}
        })).unwrap()
    }
    #[test]
    fn preserves_manual_track_and_previous_attempt_on_replay(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        upsert_run(&c,3,&run(1,"completed",Some("success"),true),"2026-11-01T00:05:00Z").unwrap();
        c.execute("INSERT INTO run_assignments VALUES(551,22,'manual',1)",[]).unwrap();
        c.execute("UPDATE workflow_runs SET resolution_status='track' WHERE run_id=551",[]).unwrap();
        upsert_run(&c,3,&run(2,"in_progress",None,false),"2026-11-01T00:06:00Z").unwrap();
        let value:(i64,String,String)=c.query_row(
            "SELECT run_attempt,pr_numbers_json,resolution_status FROM workflow_runs WHERE run_id=551",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(value.0,2);
        assert_eq!(value.1,"[9]");
        assert_eq!(value.2,"track");
        let attempts:i64=c.query_row("SELECT count(*) FROM run_attempts WHERE run_id=551",[],|r|r.get(0)).unwrap();
        assert_eq!(attempts,2);
        let manual:i64=c.query_row("SELECT manual FROM run_assignments WHERE run_id=551",[],|r|r.get(0)).unwrap();
        assert_eq!(manual,1);
    }
    #[test]
    fn retry_is_idempotent_and_keeps_unique_attempt(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        let job=run(1,"queued",None,true);
        upsert_run(&c,3,&job,"2026-11-01T00:01:00Z").unwrap();
        upsert_run(&c,3,&job,"2026-11-01T00:01:00Z").unwrap();
        let runs:i64=c.query_row("SELECT count(*) FROM workflow_runs",[],|r|r.get(0)).unwrap();
        let attempts:i64=c.query_row("SELECT count(*) FROM run_attempts",[],|r|r.get(0)).unwrap();
        assert_eq!((runs,attempts),(1,1));
    }
    #[test]
    fn nested_producer_transaction_can_upsert_without_implicit_begin(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        let outer=c.unchecked_transaction().unwrap();
        upsert_run(&outer,3,&run(1,"completed",Some("success"),true),"2026-11-01T00:05:00Z").unwrap();
        upsert_run(&outer,3,&run(2,"queued",None,true),"2026-11-01T00:06:00Z").unwrap();
        outer.commit().unwrap();
        let attempts:i64=c.query_row("SELECT count(*) FROM run_attempts WHERE run_id=551",[],|r|r.get(0)).unwrap();
        assert_eq!(attempts,2);
    }
    #[test]
    fn inner_failure_rolls_back_only_its_savepoint_not_outer_producer_work(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        let outer=c.unchecked_transaction().unwrap();
        outer.execute("INSERT INTO run_assignments VALUES(991,22,'manual',1)",[]).unwrap();
        outer.execute_batch("CREATE TRIGGER deny_attempt BEFORE INSERT ON run_attempts
            BEGIN SELECT RAISE(ABORT,'injected test failure'); END;").unwrap();
        assert!(upsert_run(&outer,3,&run(1,"completed",Some("success"),true),"2026-11-01T00:05:00Z").is_err());
        outer.commit().unwrap();
        let assigned:i64=c.query_row("SELECT count(*) FROM run_assignments WHERE run_id=991",[],|r|r.get(0)).unwrap();
        let runs:i64=c.query_row("SELECT count(*) FROM workflow_runs",[],|r|r.get(0)).unwrap();
        assert_eq!((assigned,runs),(1,0));
    }
    #[test]
    fn failed_attempt_write_rolls_back_prior_run_insert(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        c.execute_batch("CREATE TRIGGER deny_attempt BEFORE INSERT ON run_attempts
            BEGIN SELECT RAISE(ABORT,'injected test failure'); END;").unwrap();
        assert!(upsert_run(&c,3,&run(1,"completed",Some("success"),true),"2026-11-01T00:05:00Z").is_err());
        let runs:i64=c.query_row("SELECT count(*) FROM workflow_runs",[],|r|r.get(0)).unwrap();
        assert_eq!(runs,0);
    }
}
