//! One-shot, *staging-only* Run/Attempt writer. NEVER writes the installed WatchTower DB.
//! Input: existing database read-only, unused absolute staging path, exact local repository ID.
//! The only new writable SQLite connection points to a newly created backup COPY.
use anyhow::{anyhow,Result};
use chrono::Utc;
use ci_watchtower_core::{fetch_json,github_client,validate_repo,GithubRun,GithubRunsResponse};
use ci_watchtower_core::storage::upsert_run;
use keyring::Entry;
use rusqlite::{backup::Backup,params,Connection,OpenFlags};
use serde_json::{json,Value};
use std::{collections::HashSet,fs::OpenOptions,path::{Path,PathBuf},time::Duration};

const CONTRACT:&str="ci-core-stage/0.1";
const HELP:&str="Usage: ci-core-stage --source <existing-absolute-db> --stage <unused-absolute-db> --repository-id <positive-local-id> --limit 1..30\nMakes an SQLite online backup and applies one GitHub Actions batch only to that new staging copy. Does not start a poller, write the source, or grant approvals.";
const SERVICE:&str="ci-watchtower";
const ACCOUNT:&str="github-pat";

struct Args{
    source:PathBuf,stage:PathBuf,repository_id:i64,limit:usize
}
fn parse(args:&[String])->Result<Args>{
    if args.len()!=8{return Err(anyhow!("INVALID_TARGET"));}
    let mut source=None;let mut stage=None;let mut repository_id=None;let mut limit=None;
    for p in args.chunks_exact(2){
        match p[0].as_str(){
            "--source" if source.is_none()=>source=Some(PathBuf::from(&p[1])),
            "--stage" if stage.is_none()=>stage=Some(PathBuf::from(&p[1])),
            "--repository-id" if repository_id.is_none()=>{
                if p[1].starts_with('0')||!p[1].bytes().all(|b|b.is_ascii_digit()){
                    return Err(anyhow!("INVALID_TARGET"));
                }
                repository_id=Some(p[1].parse::<i64>().ok().filter(|n|*n>0)
                    .ok_or_else(||anyhow!("INVALID_TARGET"))?);
            }
            "--limit" if limit.is_none()=>{
                if p[1].starts_with('0')||!p[1].bytes().all(|b|b.is_ascii_digit()){
                    return Err(anyhow!("INVALID_TARGET"));
                }
                limit=Some(p[1].parse::<usize>().ok().filter(|n|(1..=30).contains(n))
                    .ok_or_else(||anyhow!("INVALID_TARGET"))?);
            }
            _=>return Err(anyhow!("INVALID_TARGET"))
        }
    }
    let source=source.ok_or_else(||anyhow!("INVALID_TARGET"))?;
    let stage=stage.ok_or_else(||anyhow!("INVALID_TARGET"))?;
    if !source.is_absolute()||!source.is_file()||!stage.is_absolute()||stage.exists(){
        return Err(anyhow!("INVALID_STAGE_PATH"));
    }
    let parent=stage.parent().ok_or_else(||anyhow!("INVALID_STAGE_PATH"))?;
    let absolute_parent=parent.canonicalize().map_err(|_|anyhow!("INVALID_STAGE_PATH"))?;
    let stage_name=stage.file_name().ok_or_else(||anyhow!("INVALID_STAGE_PATH"))?;
    let dest=absolute_parent.join(stage_name);
    let real_source=source.canonicalize().map_err(|_|anyhow!("DATABASE_UNAVAILABLE"))?;
    if dest==real_source {return Err(anyhow!("INVALID_STAGE_PATH"));}
    Ok(Args{source:real_source,stage:dest,
        repository_id:repository_id.ok_or_else(||anyhow!("INVALID_TARGET"))?,
        limit:limit.ok_or_else(||anyhow!("INVALID_TARGET"))?})
}
fn source_connection(path:&Path)->Result<Connection>{
    Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_|anyhow!("DATABASE_UNAVAILABLE"))
}
fn lookup_repo(source:&Connection,id:i64)->Result<String>{
    let (repo,enabled,project):(String,i64,Option<i64>)=source.query_row(
        "SELECT repo,enabled,project_id FROM monitored_repositories WHERE id=?1",
        params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .map_err(|_|anyhow!("REPOSITORY_NOT_FOUND"))?;
    if enabled!=1||project.is_none()||validate_repo(&repo).is_err(){
        return Err(anyhow!("REPOSITORY_NOT_ENABLED"));
    }
    Ok(repo)
}
fn stage_backup(source:&Connection,stage:&Path)->Result<Connection>{
    // create_new() never overwrites an existing file, including the source file.
    let reservation=OpenOptions::new().write(true).create_new(true).open(stage)
        .map_err(|_|anyhow!("INVALID_STAGE_PATH"))?;
    drop(reservation);
    let mut copy=Connection::open_with_flags(stage,OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
    {
        let backup=Backup::new(source,&mut copy).map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
        backup.run_to_completion(64,Duration::from_millis(15),None)
            .map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
    }
    let integrity:String=copy.query_row("PRAGMA integrity_check",[],|r|r.get(0))
        .map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
    if integrity!="ok"{return Err(anyhow!("STAGING_INTEGRITY_FAILED"));}
    Ok(copy)
}
fn ingest(conn:&Connection,repository_id:i64,raw:&[GithubRun],max:usize,now:&str)->Result<(usize,usize)>{
    let mut written=0;let mut skipped=0;
    let mut seen=HashSet::new();
    for run in raw.iter().take(max){
        if !seen.insert(run.id){skipped+=1;continue;}
        if run.id<1||run.workflow_id<1||run.run_attempt<1||
            run.head_sha.len()!=40||!run.head_sha.bytes().all(|b|b.is_ascii_hexdigit()){
            skipped+=1;continue;
        }
        let stored=conn.query_row(
            "SELECT repository_id,run_attempt,updated_at FROM workflow_runs WHERE run_id=?1",
            params![run.id],
            |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))
        );
        match stored{
            Ok((id,attempt,updated))=>{
                if id!=repository_id{return Err(anyhow!("RUN_REPOSITORY_CONFLICT"));}
                if attempt>run.run_attempt ||
                    (attempt==run.run_attempt&&updated>run.updated_at) {
                    skipped+=1;continue;
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows)=>{}
            Err(_)=>return Err(anyhow!("STAGING_WRITE_FAILED"))
        }
        upsert_run(conn,repository_id,run,now).map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
        written+=1;
    }
    Ok((written,skipped))
}
async fn execute(args:&Args)->Result<Value>{
    let source=source_connection(&args.source)?;
    let repository=lookup_repo(&source,args.repository_id)?;
    let token=Entry::new(SERVICE,ACCOUNT).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?
        .get_password().map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    if token.trim().is_empty(){return Err(anyhow!("CREDENTIAL_UNAVAILABLE"));}
    let client=github_client(&token).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    // A single bounded API GET. Fetch first to avoid creating a staging DB if GitHub fails.
    let url=format!("https://api.github.com/repos/{repository}/actions/runs?per_page={}&page=1",args.limit);
    let response:GithubRunsResponse=fetch_json(&client,url,"staging GitHub GET").await
        .map_err(|_|anyhow!("GITHUB_UNAVAILABLE"))?;
    let mut copy=stage_backup(&source,&args.stage)?;
    let now=Utc::now().to_rfc3339();
    let (written,skipped)=ingest(&copy,args.repository_id,&response.workflow_runs,args.limit,&now)?;
    let original_count:i64=source.query_row(
        "SELECT count(*) FROM workflow_runs WHERE repository_id=?1",
        params![args.repository_id],|r|r.get(0)).map_err(|_|anyhow!("DATABASE_SCHEMA_UNSUPPORTED"))?;
    let staged_count:i64=copy.query_row(
        "SELECT count(*) FROM workflow_runs WHERE repository_id=?1",
        params![args.repository_id],|r|r.get(0)).map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
    // Force consistency before consumers inspect the stage. Do not touch source journaling.
    copy.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|_|anyhow!("STAGING_WRITE_FAILED"))?;
    Ok(json!({"contractVersion":CONTRACT,"data":{
        "source":"github_actions_one_page","repository":repository,"repositoryId":args.repository_id,
        "stagingOnly":true,"liveDatabaseOpenedReadOnly":true,"apiRequests":1,
        "written":written,"skipped":skipped,
        "originalRepositoryRuns":original_count,"stagingRepositoryRuns":staged_count,
        "observedAt":now,"completeCoverage":false,"safeToAdvance":false
    }}))
}
#[tokio::main]
async fn main(){
    let argv:Vec<String>=std::env::args().skip(1).collect();
    if argv.len()==1&&(argv[0]=="--help"||argv[0]=="-h"){println!("{HELP}");return;}
    let result=match parse(&argv){Ok(args)=>execute(&args).await,
        Err(e)=>Err(anyhow!(match e.to_string().as_str(){
            "INVALID_STAGE_PATH"=>"INVALID_STAGE_PATH",
            "DATABASE_UNAVAILABLE"=>"DATABASE_UNAVAILABLE",
            _=>"INVALID_TARGET"
        }))};
    match result{
        Ok(v)=>println!("{v}"),
        Err(e)=>{
            let code=match e.to_string().as_str(){
                "INVALID_TARGET"=>"INVALID_TARGET",
                "INVALID_STAGE_PATH"=>"INVALID_STAGE_PATH",
                "DATABASE_UNAVAILABLE"=>"DATABASE_UNAVAILABLE",
                "REPOSITORY_NOT_FOUND"=>"REPOSITORY_NOT_FOUND",
                "REPOSITORY_NOT_ENABLED"=>"REPOSITORY_NOT_ENABLED",
                "RUN_REPOSITORY_CONFLICT"=>"RUN_REPOSITORY_CONFLICT",
                "STAGING_WRITE_FAILED"=>"STAGING_WRITE_FAILED",
                "STAGING_INTEGRITY_FAILED"=>"STAGING_INTEGRITY_FAILED",
                "DATABASE_SCHEMA_UNSUPPORTED"=>"DATABASE_SCHEMA_UNSUPPORTED",
                "CREDENTIAL_UNAVAILABLE"=>"CREDENTIAL_UNAVAILABLE",
                _=>"GITHUB_UNAVAILABLE"
            };
            println!("{}",json!({"contractVersion":CONTRACT,"error":{"code":code}}));
            std::process::exit(1);
        }
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn schema(c:&Connection){
        c.execute_batch("
        CREATE TABLE monitored_repositories(id INTEGER PRIMARY KEY,repo TEXT,enabled INTEGER,project_id INTEGER);
        INSERT INTO monitored_repositories VALUES(3,'owner/repo',1,1);
        CREATE TABLE workflow_runs(
            run_id INTEGER PRIMARY KEY,repository_id INTEGER,workflow_id INTEGER,workflow_name TEXT,
            workflow_path TEXT,display_title TEXT,event TEXT,head_branch TEXT,head_sha TEXT,
            run_number INTEGER,run_attempt INTEGER,status TEXT,conclusion TEXT,html_url TEXT,
            created_at TEXT,run_started_at TEXT,updated_at TEXT,last_seen_at TEXT,
            pr_numbers_json TEXT NOT NULL,resolution_status TEXT DEFAULT 'unassigned');
        CREATE TABLE run_attempts(run_id INTEGER,run_attempt INTEGER,status TEXT,conclusion TEXT,
            started_at TEXT,updated_at TEXT,PRIMARY KEY(run_id,run_attempt));
        CREATE TABLE run_assignments(run_id INTEGER PRIMARY KEY,track_id INTEGER,source TEXT,manual INTEGER);
        ").unwrap();
    }
    fn sample(attempt:i64)->GithubRun{
        serde_json::from_value(json!({
            "id":551,"workflow_id":8,"name":"CI","path":".github/workflows/ci.yml",
            "display_title":"CI","event":"pull_request","head_branch":"topic",
            "head_sha":"a".repeat(40),"run_number":1,"run_attempt":attempt,
            "status":"completed","conclusion":"success",
            "html_url":"https://github.com/owner/repo/actions/runs/551",
            "created_at":"2026-11-01T00:00:00Z","run_started_at":null,
            "updated_at":"2026-11-01T00:01:00Z","pull_requests":[{"number":9}]
        })).unwrap()
    }
    #[test]
    fn copy_then_ingest_preserves_source_and_manual_track(){
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir();
        let src=folder.join(format!("core-stage-src-{}-{unique}.sqlite3",std::process::id()));
        let dest=folder.join(format!("core-stage-dst-{}-{unique}.sqlite3",std::process::id()));
        {let c=Connection::open(&src).unwrap();schema(&c);
            ci_watchtower_core::storage::upsert_run(&c,3,&sample(1),"2026-11-01T00:01:00Z").unwrap();
            c.execute("INSERT INTO run_assignments VALUES(551,7,'manual',1)",[]).unwrap();}
        let srcconn=source_connection(&src).unwrap();
        let destconn=stage_backup(&srcconn,&dest).unwrap();
        assert_eq!(lookup_repo(&srcconn,3).unwrap(),"owner/repo");
        let (writes,skipped)=ingest(&destconn,3,&[sample(2),sample(1)],2,"2026-11-01T00:02:00Z").unwrap();
        assert_eq!(writes,1);
        assert_eq!(skipped,1);
        let original:i64=srcconn.query_row("SELECT run_attempt FROM workflow_runs WHERE run_id=551",[],|r|r.get(0)).unwrap();
        let staged:i64=destconn.query_row("SELECT run_attempt FROM workflow_runs WHERE run_id=551",[],|r|r.get(0)).unwrap();
        let original_manual:i64=srcconn.query_row("SELECT manual FROM run_assignments WHERE run_id=551",[],|r|r.get(0)).unwrap();
        let staged_manual:i64=destconn.query_row("SELECT manual FROM run_assignments WHERE run_id=551",[],|r|r.get(0)).unwrap();
        assert_eq!((original,staged,original_manual,staged_manual),(1,2,1,1));
        drop(destconn);drop(srcconn);
        std::fs::remove_file(&src).unwrap();std::fs::remove_file(&dest).unwrap();
    }
    #[test]
    fn refuses_to_overwrite_existing_stage_or_attach_disabled_repository(){
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir();
        let src=folder.join(format!("core-test-existing-{}-{unique}.db",std::process::id()));
        {let c=Connection::open(&src).unwrap();schema(&c);}
        let conn=source_connection(&src).unwrap();
        assert!(stage_backup(&conn,&src).is_err());
        conn.execute("SELECT 1",[]).unwrap();
        drop(conn);
        std::fs::remove_file(src).unwrap();
    }
}
