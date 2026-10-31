//! Historical SQLite evidence, not live GitHub CI verification.
//! Read-only; never initializes or upgrades the existing WatchTower database.
use anyhow::{anyhow,Result};
use chrono::Utc;
use ci_watchtower_core::{ci_kind,ci_state};
use rusqlite::{params,Connection,OpenFlags};
use serde::Serialize;
use serde_json::{json,Value};
use std::path::PathBuf;

const CONTRACT:&str="ci-core-history/0.1";
const HELP:&str="Usage: ci-core-history --db <existing-absolute-sqlite-file> --repository-id <positive-db-id> [--limit 1..100]\nHistorical WatchTower run/attempt/track evidence. Not live CI; does not grant approval.";

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct HistoryRow{
    repository_id:i64,repository:String,project_id:i64,project_key:String,
    run_id:i64,workflow_id:i64,workflow_name:String,workflow_path:Option<String>,
    ci_kind:String,head_sha:String,run_attempt:i64,latest_known_attempt:i64,
    status:String,conclusion:Option<String>,ci_state:String,
    resolution_status:String,work_track_id:Option<i64>,work_track_key:Option<String>,
    assignment_source:Option<String>,work_track_source:Option<String>,
    attribution_conflict:bool,created_at:String,updated_at:String,last_seen_at:String,
    historical_only:bool,source:String,safe_to_advance:bool
}

#[derive(Debug)]
struct Args{
    db:PathBuf,repository_id:i64,limit:i64
}
fn parse(args:&[String])->Result<Args>{
    if !(args.len()==4||args.len()==6){return Err(anyhow!("INVALID_TARGET"));}
    let mut db=None;let mut repository_id=None;let mut limit=30_i64;
    for pair in args.chunks_exact(2){
        match pair[0].as_str(){
            "--db" if db.is_none()=>{
                let path=PathBuf::from(&pair[1]);
                if !path.is_absolute()||!path.is_file(){return Err(anyhow!("DATABASE_UNAVAILABLE"));}
                db=Some(path);
            }
            "--repository-id" if repository_id.is_none()=>{
                let id=pair[1].parse::<i64>().map_err(|_|anyhow!("INVALID_TARGET"))?;
                if id<1 || pair[1].starts_with('0'){return Err(anyhow!("INVALID_TARGET"));}
                repository_id=Some(id);
            }
            "--limit" if limit==30 && pair[1]!="30"=>{
                let n=pair[1].parse::<i64>().map_err(|_|anyhow!("INVALID_TARGET"))?;
                if !(1..=100).contains(&n)||pair[1].starts_with('0'){return Err(anyhow!("INVALID_TARGET"));}
                limit=n;
            }
            _=>return Err(anyhow!("INVALID_TARGET"))
        }
    }
    Ok(Args{db:db.ok_or_else(||anyhow!("INVALID_TARGET"))?,
        repository_id:repository_id.ok_or_else(||anyhow!("INVALID_TARGET"))?,limit})
}
fn read_history(conn:&Connection,repository_id:i64,limit:i64)->Result<Value>{
    let mut stmt=conn.prepare(
        "SELECT wr.run_id,wr.workflow_id,wr.workflow_name,wr.workflow_path,
                wr.head_sha,wr.run_attempt,wr.status,wr.conclusion,
                wr.resolution_status,wr.created_at,wr.updated_at,wr.last_seen_at,
                mr.id,mr.repo,mr.project_id,p.project_key,
                ra.track_id,assigned.track_key,ra.source,ra.manual,
                assoc.track_id,work.track_key,assoc.source,
                MAX(wr.run_attempt,COALESCE((
                     SELECT MAX(a.run_attempt) FROM run_attempts a WHERE a.run_id=wr.run_id),0))
         FROM workflow_runs wr
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN projects p ON p.id=mr.project_id
         LEFT JOIN run_assignments ra ON ra.run_id=wr.run_id
         LEFT JOIN watch_tracks assigned ON assigned.id=ra.track_id
         LEFT JOIN run_track_associations assoc ON assoc.run_id=wr.run_id
         LEFT JOIN watch_tracks work ON work.id=assoc.track_id
         WHERE mr.id=?1 AND wr.ignored=0
         ORDER BY wr.created_at DESC,wr.run_id DESC LIMIT ?2")?;
    let rows=stmt.query_map(params![repository_id,limit],|r|{
        let run_id:i64=r.get(0)?;
        let workflow_id:i64=r.get(1)?;
        let workflow_name:String=r.get(2)?;
        let workflow_path:Option<String>=r.get(3)?;
        let head_sha:String=r.get(4)?;
        let run_attempt:i64=r.get(5)?;
        let status:String=r.get(6)?;
        let conclusion:Option<String>=r.get(7)?;
        let resolution_status:String=r.get(8)?;
        let created_at:String=r.get(9)?;
        let updated_at:String=r.get(10)?;
        let last_seen_at:String=r.get(11)?;
        let repository_id:i64=r.get(12)?;
        let repository:String=r.get(13)?;
        let project_id:i64=r.get(14)?;
        let project_key:String=r.get(15)?;
        let assigned_id:Option<i64>=r.get(16)?;
        let assigned_key:Option<String>=r.get(17)?;
        let assignment_source:Option<String>=r.get(18)?;
        let manual:Option<i64>=r.get(19)?;
        let associated_id:Option<i64>=r.get(20)?;
        let associated_key:Option<String>=r.get(21)?;
        let association_source:Option<String>=r.get(22)?;
        let latest_known_attempt:i64=r.get(23)?;
        let is_manual=manual==Some(1);
        let (track_id,track_key,work_source)=if is_manual{
            (assigned_id,assigned_key,assignment_source.clone())
        }else{
            (associated_id.or(assigned_id),associated_key.or(assigned_key),association_source.or(assignment_source.clone()))
        };
        Ok(HistoryRow{
            repository_id,repository,project_id,project_key,run_id,workflow_id,
            workflow_name,workflow_path:workflow_path.clone(),
            ci_kind:ci_kind(workflow_path.as_deref()).into(),head_sha,run_attempt,
            latest_known_attempt,status:status.clone(),conclusion:conclusion.clone(),
            ci_state:ci_state(&status,conclusion.as_deref()).into(),
            resolution_status:resolution_status.clone(),work_track_id:track_id,
            work_track_key:track_key,assignment_source,work_track_source:work_source,
            attribution_conflict:resolution_status=="conflict",created_at,updated_at,last_seen_at,
            historical_only:true,source:"watchtower_sqlite_history".into(),safe_to_advance:false
        })
    })?;
    let runs=rows.collect::<rusqlite::Result<Vec<_>>>()?;
    // Bounded historical rows can never establish the latest effective CI check set.
    Ok(json!({"contractVersion":CONTRACT,"data":{
        "source":"watchtower_sqlite_history","observedAt":Utc::now().to_rfc3339(),
        "repositoryId":repository_id,"historicalOnly":true,
        "completeCoverage":false,"safeToAdvance":false,"runs":runs
    }}))
}
fn run(args:&[String])->Result<Value>{
    let a=parse(args)?;
    let c=Connection::open_with_flags(&a.db,OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_|anyhow!("DATABASE_UNAVAILABLE"))?;
    read_history(&c,a.repository_id,a.limit)
        .map_err(|_|anyhow!("DATABASE_SCHEMA_UNSUPPORTED"))
}
fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    if args.len()==1 && matches!(args[0].as_str(),"-h"|"--help"){
        println!("{HELP}");return;
    }
    match run(&args){
        Ok(o)=>println!("{o}"),
        Err(err)=>{
            let code=match err.to_string().as_str(){
                "INVALID_TARGET"=>"INVALID_TARGET",
                "DATABASE_UNAVAILABLE"=>"DATABASE_UNAVAILABLE",
                _=>"DATABASE_SCHEMA_UNSUPPORTED"
            };
            println!("{}",json!({"contractVersion":CONTRACT,"error":{"code":code}}));
            std::process::exit(1);
        }
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn fixture(conn:&Connection){
        conn.execute_batch("
            CREATE TABLE projects(id INTEGER PRIMARY KEY,project_key TEXT);
            CREATE TABLE monitored_repositories(id INTEGER PRIMARY KEY,repo TEXT,project_id INTEGER);
            CREATE TABLE watch_tracks(id INTEGER PRIMARY KEY,track_key TEXT);
            CREATE TABLE workflow_runs(run_id INTEGER PRIMARY KEY,repository_id INTEGER,workflow_id INTEGER,
              workflow_name TEXT,workflow_path TEXT,head_sha TEXT,run_attempt INTEGER,status TEXT,
              conclusion TEXT,resolution_status TEXT,created_at TEXT,updated_at TEXT,last_seen_at TEXT,
              ignored INTEGER);
            CREATE TABLE run_attempts(run_id INTEGER,run_attempt INTEGER);
            CREATE TABLE run_assignments(run_id INTEGER,track_id INTEGER,source TEXT,manual INTEGER);
            CREATE TABLE run_track_associations(run_id INTEGER,track_id INTEGER,source TEXT);
            INSERT INTO projects VALUES(1,'project-a');
            INSERT INTO monitored_repositories VALUES(2,'owner/repo',1);
            INSERT INTO watch_tracks VALUES(3,'ops');
            INSERT INTO workflow_runs VALUES(7,2,5,'CI','.github/workflows/ci.yml',
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,'completed','success','resolved',
              '2026-11-01T00:00:00Z','2026-11-01T00:03:00Z','2026-11-01T00:03:00Z',0);
            INSERT INTO run_attempts VALUES(7,1);INSERT INTO run_attempts VALUES(7,2);
            INSERT INTO run_assignments VALUES(7,3,'manual',1);
        ").unwrap();
    }
    #[test]
    fn preserves_historical_attempt_and_manual_track_but_never_grants_approval(){
        let c=Connection::open_in_memory().unwrap();fixture(&c);
        let d=read_history(&c,2,10).unwrap();
        assert_eq!(d["data"]["runs"].as_array().unwrap().len(),1);
        let r=&d["data"]["runs"][0];
        assert_eq!(r["ciState"],"SUCCESS");
        assert_eq!(r["latestKnownAttempt"],2);
        assert_eq!(r["workTrackKey"],"ops");
        assert_eq!(r["workTrackId"],3);
        assert_eq!(r["safeToAdvance"],false);
        assert_eq!(d["data"]["safeToAdvance"],false);
        assert_eq!(d["data"]["historicalOnly"],true);
        assert_eq!(d["data"]["completeCoverage"],false);
    }
    #[test]
    fn rejects_invalid_input_and_unknown_schema_without_creating_file(){
        assert!(parse(&[]).is_err());
        let path=std::env::temp_dir().join("ci-watchtower-missing-registry-20261101.sqlite3");
        if !path.exists(){
            assert!(parse(&["--db".into(),path.to_string_lossy().into(),
                "--repository-id".into(),"2".into()]).is_err());
        }
        assert!(read_history(&Connection::open_in_memory().unwrap(),1,3).is_err());
    }
    #[test]
    fn read_only_file_never_allows_a_write(){
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("ci-history-{}-{unique}.db",std::process::id()));
        {let conn=Connection::open(&path).unwrap();fixture(&conn);}
        let conn=Connection::open_with_flags(&path,OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(read_history(&conn,2,1).unwrap()["data"]["runs"].as_array().unwrap().len(),1);
        assert!(conn.execute("DELETE FROM workflow_runs",[]).is_err());
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }
}
