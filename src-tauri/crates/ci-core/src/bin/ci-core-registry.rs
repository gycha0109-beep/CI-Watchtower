//! Read-only, UI-free registry inspection of an existing WatchTower database.
//! Never initializes, upgrades, migrates, replaces or writes to the database.
use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;

const CONTRACT:&str="ci-core-registry/0.1";
const HELP:&str="Usage: ci-core-registry --db <existing-absolute-path-to-ci-watchtower.sqlite3>\nReads the registry without creating or updating the database. Advisory only; never authorizes automatic continuation.";

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct RegistryProject{
    id:i64,key:String,name:String,active:bool
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct RegistryRepository{
    id:i64,project_id:Option<i64>,name:String,enabled:bool,
    last_poll_attempt_at:Option<String>,last_successful_poll_at:Option<String>,
    poll_error_code:Option<String>
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct RegistryTrack{
    id:i64,project_id:i64,canonical_track_key:String,name:String,active:bool
}

fn open_existing_readonly(path:&PathBuf)->Result<Connection>{
    if !path.is_absolute()||!path.is_file(){return Err(anyhow!("DATABASE_UNAVAILABLE"));}
    Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_|anyhow!("DATABASE_UNAVAILABLE"))
}

fn registry(conn:&Connection)->Result<Value>{
    let projects:Vec<RegistryProject>={
        let mut stmt=conn.prepare("SELECT id,project_key,name,active FROM projects ORDER BY id")?;
        let rows=stmt.query_map([],|row|Ok(RegistryProject{
            id:row.get(0)?,key:row.get(1)?,name:row.get(2)?,
            active:row.get::<_,i64>(3)?!=0
        }))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let repositories:Vec<RegistryRepository>={
        let mut stmt=conn.prepare(
            "SELECT id,project_id,repo,enabled,last_polled_at,last_successful_poll_at,last_error
               FROM monitored_repositories ORDER BY repo")?;
        let rows=stmt.query_map([],|row|{
            let err:Option<String>=row.get(6)?;
            Ok(RegistryRepository{
                id:row.get(0)?,project_id:row.get(1)?,name:row.get(2)?,
                enabled:row.get::<_,i64>(3)?!=0,last_poll_attempt_at:row.get(4)?,
                last_successful_poll_at:row.get(5)?,
                // Never expose raw GitHub API messages or potentially sensitive error bodies.
                poll_error_code:err.filter(|s|!s.trim().is_empty()).map(|_|"POLL_ERROR".into())
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let tracks:Vec<RegistryTrack>={
        let mut stmt=conn.prepare(
            "SELECT id,project_id,track_key,name,active FROM watch_tracks ORDER BY id")?;
        let rows=stmt.query_map([],|row|Ok(RegistryTrack{
            id:row.get(0)?,project_id:row.get(1)?,
            canonical_track_key:row.get(2)?,name:row.get(3)?,
            active:row.get::<_,i64>(4)?!=0
        }))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(json!({"contractVersion":CONTRACT,"data":{
        "source":"watchtower_existing_sqlite",
        "observedAt":Utc::now().to_rfc3339(),
        "readOnly":true,"completeCoverage":false,"safeToAdvance":false,
        "projects":projects,"repositories":repositories,"tracks":tracks
    }}))
}

fn run(args:&[String])->Result<Value>{
    if args.len()!=2||args[0]!="--db"{return Err(anyhow!("INVALID_TARGET"));}
    let path=PathBuf::from(&args[1]);
    let conn=open_existing_readonly(&path)?;
    registry(&conn).map_err(|_|anyhow!("DATABASE_SCHEMA_UNSUPPORTED"))
}

fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    if args.len()==1&&(args[0]=="-h"||args[0]=="--help"){
        println!("{HELP}");return;
    }
    match run(&args){
        Ok(value)=>println!("{value}"),
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
    fn schema(conn:&Connection){
        conn.execute_batch("
            CREATE TABLE projects(id INTEGER PRIMARY KEY,project_key TEXT,name TEXT,active INTEGER);
            CREATE TABLE monitored_repositories(
                id INTEGER PRIMARY KEY,project_id INTEGER,repo TEXT,enabled INTEGER,
                last_polled_at TEXT,last_successful_poll_at TEXT,last_error TEXT);
            CREATE TABLE watch_tracks(
                id INTEGER PRIMARY KEY,project_id INTEGER,track_key TEXT,name TEXT,active INTEGER);
            INSERT INTO projects VALUES(1,'a','Alpha',1);
            INSERT INTO monitored_repositories VALUES(2,1,'owner/repo',1,NULL,'2026-11-01T00:00:00Z','GitHub: secret failure');
            INSERT INTO watch_tracks VALUES(3,1,'ops','Operations',1);
        ").unwrap();
    }
    #[test]
    fn registry_retains_ids_and_filters_raw_errors(){
        let c=Connection::open_in_memory().unwrap();schema(&c);
        let result=registry(&c).unwrap();
        assert_eq!(result["contractVersion"],CONTRACT);
        assert_eq!(result["data"]["projects"][0]["key"],"a");
        assert_eq!(result["data"]["repositories"][0]["projectId"],1);
        assert_eq!(result["data"]["repositories"][0]["pollErrorCode"],"POLL_ERROR");
        assert_eq!(result["data"]["tracks"][0]["canonicalTrackKey"],"ops");
        assert_eq!(result["data"]["readOnly"],true);
        assert_eq!(result["data"]["safeToAdvance"],false);
        assert!(!result.to_string().contains("secret failure"));
    }
    #[test]
    fn existing_file_is_opened_without_write_permission(){
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("ci-watchtower-registry-{}-{unique}.sqlite3",std::process::id()));
        {
            let conn=Connection::open(&path).unwrap();
            schema(&conn);
        }
        {
            let read=open_existing_readonly(&path).unwrap();
            assert_eq!(registry(&read).unwrap()["data"]["tracks"].as_array().unwrap().len(),1);
            assert!(read.execute("DELETE FROM projects",[]).is_err());
        }
        std::fs::remove_file(&path).unwrap();
        assert!(open_existing_readonly(&path).is_err());
    }
    #[test]
    fn reject_paths_and_unsupported_schema(){
        assert!(run(&[]).is_err());
        assert!(run(&["--db".into(),"relative.sqlite3".into()]).is_err());
        assert!(registry(&Connection::open_in_memory().unwrap()).is_err());
    }
}
