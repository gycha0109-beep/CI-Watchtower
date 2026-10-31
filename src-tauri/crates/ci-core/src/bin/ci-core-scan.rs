//! Live but *incomplete* GitHub Actions observation, independent of WatchTower Tauri.
//! No SQLite writes, automatic approval, implicit polling schedule or PAT output.
use anyhow::{anyhow,Result};
use chrono::Utc;
use ci_watchtower_core::{ci_kind,ci_state,github_client,integration_candidate,validate_repo,GithubRun,GithubRunsResponse};
use keyring::Entry;
use reqwest::{Client,StatusCode};
use serde::{de::DeserializeOwned,Serialize};
use serde_json::{json,Value};
use std::collections::HashMap;

const CONTRACT:&str="ci-core-scan/0.1";
const HELP:&str="Usage: ci-core-scan --repository owner/repo --limit 1..50\nOne-shot bounded GitHub Actions observation (max six API GETs). Not complete CI evidence or approval.";
const SERVICE:&str="ci-watchtower";
const ACCOUNT:&str="github-pat";
const ACTIVE:&[&str]=&["queued","in_progress","waiting","pending","requested"];

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct ScannedRun{
    run_id:i64,workflow_id:i64,workflow_name:String,workflow_path:Option<String>,
    ci_kind:String,ci_state:String,status:String,conclusion:Option<String>,
    head_sha:String,head_branch:Option<String>,run_attempt:i64,run_number:i64,
    pr_numbers:Vec<i64>,integration_candidate_head_sha:Option<String>,
    created_at:String,updated_at:String,
    target_verified:bool,required_checks_verified:bool,safe_to_advance:bool
}
fn parse(args:&[String])->Result<(String,usize)>{
    if args.len()!=4{return Err(anyhow!("INVALID_TARGET"));}
    let mut repository=None;let mut limit=None;
    for pair in args.chunks_exact(2){
        match pair[0].as_str(){
            "--repository" if repository.is_none()=>{
                validate_repo(&pair[1]).map_err(|_|anyhow!("INVALID_TARGET"))?;
                repository=Some(pair[1].clone());
            }
            "--limit" if limit.is_none()=>{
                if pair[1].starts_with('0')||!pair[1].bytes().all(|b|b.is_ascii_digit()){
                    return Err(anyhow!("INVALID_TARGET"));
                }
                let n=pair[1].parse::<usize>().map_err(|_|anyhow!("INVALID_TARGET"))?;
                if !(1..=50).contains(&n){return Err(anyhow!("INVALID_TARGET"));}
                limit=Some(n);
            }
            _=>return Err(anyhow!("INVALID_TARGET"))
        }
    }
    Ok((repository.ok_or_else(||anyhow!("INVALID_TARGET"))?,
        limit.ok_or_else(||anyhow!("INVALID_TARGET"))?))
}
fn status_code(status:StatusCode,remaining:Option<&str>,retry:bool)->&'static str{
    match status.as_u16(){
        401=>"GITHUB_UNAUTHORIZED",
        403 if remaining==Some("0")=>"GITHUB_RATE_LIMITED",
        403 if retry=>"GITHUB_THROTTLED",
        403=>"GITHUB_FORBIDDEN",
        404=>"GITHUB_NOT_FOUND",
        429=>"GITHUB_THROTTLED",
        _=>"GITHUB_UNAVAILABLE"
    }
}
async fn read<T:DeserializeOwned>(client:&Client,url:String)->Result<T>{
    let response=client.get(url).send().await
        .map_err(|_|anyhow!("GITHUB_TRANSPORT_UNAVAILABLE"))?;
    if !response.status().is_success(){
        let code=status_code(response.status(),
            response.headers().get("x-ratelimit-remaining").and_then(|v|v.to_str().ok()),
            response.headers().contains_key("retry-after"));
        return Err(anyhow!(code));
    }
    response.json::<T>().await.map_err(|_|anyhow!("GITHUB_RESPONSE_INVALID"))
}
fn limit_runs(mut raw:Vec<GithubRun>,limit:usize)->Vec<ScannedRun>{
    // The same run may appear in the general page and an active-status page.
    let mut dedup:HashMap<i64,GithubRun>=HashMap::new();
    for run in raw.drain(..){
        if run.id<1||run.run_attempt<1||run.head_sha.len()!=40||
            !run.head_sha.bytes().all(|b|b.is_ascii_hexdigit()){continue;}
        match dedup.get(&run.id){
            Some(previous) if previous.run_attempt>run.run_attempt=>continue,
            Some(previous) if previous.run_attempt==run.run_attempt && previous.updated_at>run.updated_at=>continue,
            _=>{}
        }
        dedup.insert(run.id,run);
    }
    let mut runs:Vec<GithubRun>=dedup.into_values().collect();
    runs.sort_by(|a,b|ACTIVE.contains(&b.status.as_str()).cmp(&ACTIVE.contains(&a.status.as_str()))
        .then_with(||b.created_at.cmp(&a.created_at)).then_with(||b.id.cmp(&a.id)));
    runs.into_iter().take(limit).map(|r|{
        let candidate=integration_candidate(r.path.as_deref(),r.display_title.as_deref().unwrap_or(&r.name));
        ScannedRun{
            run_id:r.id,workflow_id:r.workflow_id,workflow_name:r.name,
            ci_kind:ci_kind(r.path.as_deref()).into(),workflow_path:r.path,
            ci_state:ci_state(&r.status,r.conclusion.as_deref()).into(),
            status:r.status,conclusion:r.conclusion,
            head_sha:r.head_sha,head_branch:r.head_branch,
            run_attempt:r.run_attempt,run_number:r.run_number,
            pr_numbers:r.pull_requests.into_iter().map(|p|p.number).collect(),
            integration_candidate_head_sha:candidate.map(|(_,head)|head),
            created_at:r.created_at,updated_at:r.updated_at,
            target_verified:false,required_checks_verified:false,safe_to_advance:false
        }
    }).collect()
}
async fn scan(repository:&str,limit:usize)->Result<Value>{
    let token=Entry::new(SERVICE,ACCOUNT).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?
        .get_password().map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    if token.trim().is_empty(){return Err(anyhow!("CREDENTIAL_UNAVAILABLE"));}
    let client=github_client(&token).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    let base=format!("https://api.github.com/repos/{repository}/actions/runs");
    let recent:GithubRunsResponse=read(&client,format!("{base}?per_page=100&page=1")).await?;
    let mut all=recent.workflow_runs;
    // Explicitly bounded one page per active status; avoids runaway API calls.
    for status in ACTIVE{
        let page:GithubRunsResponse=read(&client,format!("{base}?status={status}&per_page=100&page=1")).await?;
        all.extend(page.workflow_runs);
    }
    let runs=limit_runs(all,limit);
    Ok(json!({"contractVersion":CONTRACT,"data":{
        "source":"github_actions_recent","repository":repository,
        "observedAt":Utc::now().to_rfc3339(),
        "apiRequests":1+ACTIVE.len(),"maxApiRequests":6,
        "sampleLimit":limit,"completeCoverage":false,"targetVerified":false,
        "requiredChecksVerified":false,"safeToAdvance":false,
        "runs":runs
    }}))
}
#[tokio::main]
async fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    if args.len()==1 && (args[0]=="--help"||args[0]=="-h"){
        println!("{HELP}");return;
    }
    let res=match parse(&args){
        Ok((repo,limit))=>scan(&repo,limit).await,
        Err(_)=>Err(anyhow!("INVALID_TARGET"))
    };
    match res{
        Ok(value)=>println!("{value}"),
        Err(err)=>{
            let code=match err.to_string().as_str(){
                "INVALID_TARGET"=>"INVALID_TARGET",
                "CREDENTIAL_UNAVAILABLE"=>"CREDENTIAL_UNAVAILABLE",
                "GITHUB_UNAUTHORIZED"=>"GITHUB_UNAUTHORIZED",
                "GITHUB_FORBIDDEN"=>"GITHUB_FORBIDDEN",
                "GITHUB_RATE_LIMITED"=>"GITHUB_RATE_LIMITED",
                "GITHUB_THROTTLED"=>"GITHUB_THROTTLED",
                "GITHUB_NOT_FOUND"=>"GITHUB_NOT_FOUND",
                "GITHUB_TRANSPORT_UNAVAILABLE"=>"GITHUB_TRANSPORT_UNAVAILABLE",
                "GITHUB_RESPONSE_INVALID"=>"GITHUB_RESPONSE_INVALID",
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
    fn run(id:i64,attempt:i64,status:&str,conclusion:Option<&str>)->GithubRun{
        serde_json::from_value(json!({
            "id":id,"workflow_id":11,"name":"CI","path":".github/workflows/ci.yml",
            "display_title":"CI","event":"pull_request","head_branch":"feat/x",
            "head_sha":"a".repeat(40),"run_number":1,"run_attempt":attempt,
            "status":status,"conclusion":conclusion,
            "html_url":"https://github.com/owner/repo/actions/runs/1",
            "created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-01T00:02:00Z",
            "pull_requests":[{"number":9}]
        })).unwrap()
    }
    #[test]
    fn parser_rejects_repo_injection_unbounded_limits_and_duplicates(){
        let ok=&["--repository".into(),"owner/repo".into(),"--limit".into(),"50".into()];
        assert!(parse(ok).is_ok());
        assert!(parse(&["--repository".into(),"owner/repo?x=1".into(),"--limit".into(),"3".into()]).is_err());
        assert!(parse(&["--repository".into(),"owner/repo".into(),"--limit".into(),"51".into()]).is_err());
        assert!(parse(&["--repository".into(),"owner/repo".into(),"--repository".into(),"owner/other".into()]).is_err());
    }
    #[test]
    fn duplicate_run_keeps_newest_attempt_and_no_approval(){
        let list=limit_runs(vec![run(7,1,"completed",Some("success")),
            run(7,2,"in_progress",None),run(8,1,"completed",Some("failure"))],10);
        assert_eq!(list.len(),2);
        let selected=list.iter().find(|r|r.run_id==7).unwrap();
        assert_eq!(selected.run_attempt,2);
        assert_eq!(selected.ci_state,"RUNNING");
        assert!(!selected.target_verified);
        assert!(!selected.safe_to_advance);
    }
    #[test]
    fn long_running_workflow_survives_recent_completed_run_window(){
        let mut old=run(7,1,"in_progress",None);
        old.created_at="2020-01-01T00:00:00Z".into();
        let recent=run(8,1,"completed",Some("success"));
        let rows=limit_runs(vec![old,recent],1);
        assert_eq!(rows.len(),1);
        assert_eq!(rows[0].run_id,7);
        assert_eq!(rows[0].ci_state,"RUNNING");
        assert!(!rows[0].safe_to_advance);
    }
    #[test]
    fn refuse_token_leaking_status_diagnostics(){
        assert_eq!(status_code(StatusCode::FORBIDDEN,Some("0"),false),"GITHUB_RATE_LIMITED");
        assert_eq!(status_code(StatusCode::UNAUTHORIZED,None,false),"GITHUB_UNAUTHORIZED");
        assert_eq!(status_code(StatusCode::FORBIDDEN,None,false),"GITHUB_FORBIDDEN");
    }
}
