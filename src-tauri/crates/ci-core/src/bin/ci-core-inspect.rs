//! One-shot, read-only headless GitHub CI inspection.
//! No WatchTower UI, SQLite mutations, webhook, merge or automatic resume.
use ci_watchtower_core::{ci_kind, ci_state, github_client, integration_candidate, validate_repo, GithubRun};
use anyhow::{anyhow, Result};
use chrono::Utc;
use keyring::Entry;
use serde::{Deserialize, de::DeserializeOwned};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;

const SERVICE: &str = "ci-watchtower";
const ACCOUNT: &str = "github-pat";
const CONTRACT: &str = "ci-core-inspect/0.1";
const HELP: &str = "Usage: ci-core-inspect --repository owner/repo --pr 123 --head-sha <40hex> --run-id 456 --attempt 1\nReads existing Windows Credential Manager GitHub PAT. Read-only; never authorizes a merge or resume.";

#[derive(Clone, Debug)]
struct Target {
    repository: String,
    pr: i64,
    head: String,
    run_id: i64,
    attempt: i64,
}

#[derive(Deserialize)]
struct PullHead { sha: String }

#[derive(Deserialize)]
struct PullRepo { full_name: String }

#[derive(Deserialize)]
struct PullBase { repo: PullRepo }

#[derive(Deserialize)]
struct GithubPr {
    number: i64,
    head: PullHead,
    base: PullBase,
}

fn target_from_args(args: &[String]) -> Result<Target> {
    if args.len() != 10 { return Err(anyhow!("INVALID_TARGET")); }
    let mut flags=HashMap::new();
    for pair in args.chunks_exact(2) {
        if !matches!(pair[0].as_str(), "--repository"|"--pr"|"--head-sha"|"--run-id"|"--attempt")
            || flags.insert(pair[0].as_str(),pair[1].as_str()).is_some() {
            return Err(anyhow!("INVALID_TARGET"));
        }
    }
    let repository=flags.get("--repository").ok_or_else(||anyhow!("INVALID_TARGET"))?.to_string();
    validate_repo(&repository).map_err(|_|anyhow!("INVALID_TARGET"))?;
    let head=flags.get("--head-sha").ok_or_else(||anyhow!("INVALID_TARGET"))?.to_string();
    if head.len()!=40 || !head.bytes().all(|b|b.is_ascii_hexdigit()) { return Err(anyhow!("INVALID_TARGET")); }
    let number=|flag| -> Result<i64> {
        let raw=flags.get(flag).ok_or_else(||anyhow!("INVALID_TARGET"))?;
        if raw.starts_with('0') || !raw.bytes().all(|b|b.is_ascii_digit()) {
            return Err(anyhow!("INVALID_TARGET"));
        }
        raw.parse::<i64>().ok().filter(|n|*n>0).ok_or_else(||anyhow!("INVALID_TARGET"))
    };
    Ok(Target { repository,pr:number("--pr")?,head:head.to_ascii_lowercase(),
        run_id:number("--run-id")?,attempt:number("--attempt")? })
}

// This is a single-run observation, NOT the required-check set or project attribution.
// Integration workflow_dispatch runs often execute on the default branch SHA,
// while their title claims a different candidate PR SHA. Do not swap these fields.
fn assess(target: &Target, before:&GithubPr, after:&GithubPr, run:&GithubRun) -> Value {
    let raw=ci_state(&run.status,run.conclusion.as_deref());
    let kind=ci_kind(run.path.as_deref());
    let candidate=integration_candidate(run.path.as_deref(),
        run.display_title.as_deref().unwrap_or(&run.name));
    let pr_stable=before.number==target.pr && after.number==target.pr &&
        before.base.repo.full_name==target.repository && after.base.repo.full_name==target.repository &&
        before.head.sha.eq_ignore_ascii_case(&target.head) &&
        after.head.sha.eq_ignore_ascii_case(&target.head);
    let run_stable=run.id==target.run_id && run.run_attempt==target.attempt;
    let via_pr=run.pull_requests.iter().any(|p|p.number==target.pr) &&
        run.head_sha.eq_ignore_ascii_case(&target.head);
    let via_integration=kind=="integration" &&
        candidate.as_ref().is_some_and(|(number,head)|*number==target.pr && head==&target.head);
    let source=if via_pr {"github_pull_requests"}
        else if via_integration {"integration_title_candidate_only"} else {"unverified"};
    let identity=if !pr_stable {"PR_HEAD_OR_REPOSITORY_MISMATCH"}
        else if !run_stable {"RUN_ID_OR_ATTEMPT_MISMATCH"}
        else if !via_pr && !via_integration {"PR_RUN_ASSOCIATION_UNVERIFIED"}
        else {"MATCH"};
    let general_verified=identity=="MATCH" && via_pr && kind!="integration";
    // Integration's pinned base/admission and complete check set are not available here.
    let decision=if identity=="PR_HEAD_OR_REPOSITORY_MISMATCH" ||
        identity=="RUN_ID_OR_ATTEMPT_MISMATCH" {"CONFLICT"}
        else if !general_verified {"UNKNOWN"}
        else {raw};
    json!({
        "contractVersion": CONTRACT,
        "data": {
            "source": "github_actions_direct",
            "observedAt": Utc::now().to_rfc3339(),
            "repository": target.repository,
            "prNumber": target.pr,
            "expectedHeadSha": target.head,
            "currentPrHeadSha": after.head.sha,
            "runId": run.id,
            "runAttempt": run.run_attempt,
            "workflowPath": run.path,
            "ciKind": kind,
            "workflowRunHeadSha": run.head_sha,
            "integrationCandidateHeadSha": candidate.map(|(_,head)|head),
            "integrationBaselineVerified": false,
            "prIdentitySource": source,
            "targetIdentityState": identity,
            "targetVerified": general_verified,
            "status": run.status,
            "conclusion": run.conclusion,
            "ciState": raw,
            "decisionState": decision,
            "requiredChecksVerified": false,
            "completeCoverage": false,
            "safeToAdvance": false
        }
    })
}

// Return only fixed, non-sensitive error codes. Never disclose the PAT,
// HTTP response body, request headers or GitHub's diagnostic text.
fn github_error(status:StatusCode, remaining:Option<&str>, retry_after:bool)->&'static str{
    match status.as_u16(){
        401=>"GITHUB_UNAUTHORIZED",
        403 if remaining==Some("0")=>"GITHUB_RATE_LIMITED",
        403 if retry_after=>"GITHUB_THROTTLED",
        403=>"GITHUB_FORBIDDEN",
        404=>"GITHUB_NOT_FOUND",
        429=>"GITHUB_THROTTLED",
        _=>"GITHUB_UNAVAILABLE"
    }
}

async fn read_github<T:DeserializeOwned>(client:&Client,url:String)->Result<T>{
    let reply=client.get(url).send().await
        .map_err(|_|anyhow!("GITHUB_TRANSPORT_UNAVAILABLE"))?;
    if !reply.status().is_success(){
        let code=github_error(reply.status(),
            reply.headers().get("x-ratelimit-remaining").and_then(|v|v.to_str().ok()),
            reply.headers().contains_key("retry-after"));
        return Err(anyhow!(code));
    }
    reply.json::<T>().await.map_err(|_|anyhow!("GITHUB_RESPONSE_INVALID"))
}

async fn inspect(target:&Target)->Result<Value>{
    let token=Entry::new(SERVICE,ACCOUNT).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?
        .get_password().map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    if token.trim().is_empty() { return Err(anyhow!("CREDENTIAL_UNAVAILABLE")); }
    let client=github_client(&token).map_err(|_|anyhow!("CREDENTIAL_UNAVAILABLE"))?;
    let base=format!("https://api.github.com/repos/{}",target.repository);
    let pr_url=format!("{base}/pulls/{}",target.pr);
    let run_url=format!("{base}/actions/runs/{}",target.run_id);
    // Read twice around run retrieval to reject a moving PR head.
    let before:GithubPr=read_github(&client,pr_url.clone()).await?;
    let run:GithubRun=read_github(&client,run_url).await?;
    let after:GithubPr=read_github(&client,pr_url).await?;
    Ok(assess(target,&before,&after,&run))
}

#[tokio::main]
async fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    if args.len()==1 && (args[0]=="--help" || args[0]=="-h"){
        println!("{HELP}");
        return;
    }
    let result=match target_from_args(&args){
        Ok(target)=>inspect(&target).await,
        Err(_)=>Err(anyhow!("INVALID_TARGET"))
    };
    match result{
        Ok(data)=>println!("{}",data),
        Err(error)=>{
            let code=match error.to_string().as_str(){
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
    fn target()->Target{Target{repository:"owner/repo".into(),pr:9,
        head:"a".repeat(40),run_id:123,attempt:2}}
    fn pr(sha:&str)->GithubPr{GithubPr{
        number:9,head:PullHead{sha:sha.into()},
        base:PullBase{repo:PullRepo{full_name:"owner/repo".into()}}}}
    fn run(status:&str,conclusion:Option<&str>,kind:&str)->GithubRun {
        serde_json::from_value(json!({
            "id":123,"workflow_id":1,"name":"CI",
            "path":format!(".github/workflows/{kind}"),
            "display_title":"CI","event":"pull_request",
            "head_branch":"feat/9","head_sha":"a".repeat(40),"run_number":1,
            "run_attempt":2,"status":status,"conclusion":conclusion,
            "html_url":"https://github.com/owner/repo/actions/runs/123",
            "created_at":"2026-11-01T00:00:00Z","run_started_at":null,
            "updated_at":"2026-11-01T00:01:00Z","pull_requests":[{"number":9}]
        })).unwrap()
    }
    #[test]
    fn classify_http_without_logging_credentials_or_github_response(){
        assert_eq!(github_error(StatusCode::UNAUTHORIZED,None,false),"GITHUB_UNAUTHORIZED");
        assert_eq!(github_error(StatusCode::FORBIDDEN,Some("0"),false),"GITHUB_RATE_LIMITED");
        assert_eq!(github_error(StatusCode::FORBIDDEN,Some("50"),true),"GITHUB_THROTTLED");
        assert_eq!(github_error(StatusCode::FORBIDDEN,Some("50"),false),"GITHUB_FORBIDDEN");
        assert_eq!(github_error(StatusCode::NOT_FOUND,None,false),"GITHUB_NOT_FOUND");
        assert_eq!(github_error(StatusCode::TOO_MANY_REQUESTS,None,false),"GITHUB_THROTTLED");
        assert_eq!(github_error(StatusCode::BAD_GATEWAY,None,false),"GITHUB_UNAVAILABLE");
    }
    #[test]
    fn malformed_scope_or_duplicate_flags_never_start_network(){
        assert!(target_from_args(&vec![]).is_err());
        let mut args=vec!["--repository","owner/repo","--pr","9","--head-sha",
            "a", "--run-id","123","--attempt","2"].into_iter().map(String::from).collect::<Vec<_>>();
        assert!(target_from_args(&args).is_err());
        args[5]="a".repeat(40);
        assert_eq!(target_from_args(&args).unwrap().attempt,2);
        args[0]="--run-id".into();
        assert!(target_from_args(&args).is_err());
    }
    #[test]
    fn observed_success_always_requires_independent_pr_and_check_approval(){
        let t=target();let good=run("completed",Some("success"),"ci.yml");
        let proof=assess(&t,&pr(&t.head),&pr(&t.head),&good);
        assert_eq!(proof["data"]["ciState"],"SUCCESS");
        assert_eq!(proof["data"]["decisionState"],"SUCCESS");
        assert_eq!(proof["data"]["targetVerified"],true);
        assert_eq!(proof["data"]["safeToAdvance"],false);
        assert_eq!(proof["data"]["completeCoverage"],false);
        let drift=assess(&t,&pr(&t.head),&pr(&"b".repeat(40)),&good);
        assert_eq!(drift["data"]["decisionState"],"CONFLICT");
        let old_attempt=assess(&Target{attempt:3,..t.clone()},&pr(&t.head),&pr(&t.head),&good);
        assert_eq!(old_attempt["data"]["decisionState"],"CONFLICT");
    }
    #[test]
    fn integration_candidate_is_not_verified_without_pinned_base(){
        let t=target();let mut job=run("completed",Some("success"),"ci-integration.yml");
        job.head_sha="b".repeat(40);
        job.pull_requests.clear();
        job.display_title=Some(format!("Integration PR #9 at {}",t.head));
        let data=assess(&t,&pr(&t.head),&pr(&t.head),&job);
        assert_eq!(data["data"]["ciKind"],"integration");
        assert_eq!(data["data"]["ciState"],"SUCCESS");
        assert_eq!(data["data"]["targetIdentityState"],"MATCH");
        assert_eq!(data["data"]["targetVerified"],false);
        assert_eq!(data["data"]["integrationBaselineVerified"],false);
        assert_eq!(data["data"]["decisionState"],"UNKNOWN");
    }
    #[test]
    fn integration_even_with_pr_association_is_never_a_verified_general_run(){
        let t=target();let mut job=run("completed",Some("success"),"ci-integration.yml");
        job.display_title=Some(format!("Integration PR #9 at {}",t.head));
        let data=assess(&t,&pr(&t.head),&pr(&t.head),&job);
        assert_eq!(data["data"]["targetIdentityState"],"MATCH");
        assert_eq!(data["data"]["targetVerified"],false);
        assert_eq!(data["data"]["decisionState"],"UNKNOWN");
    }
    #[test]
    fn unknown_pr_run_association_never_uses_historical_green(){
        let t=target();let mut job=run("completed",Some("success"),"ci.yml");
        job.pull_requests.clear();
        let data=assess(&t,&pr(&t.head),&pr(&t.head),&job);
        assert_eq!(data["data"]["decisionState"],"UNKNOWN");
        assert_eq!(data["data"]["safeToAdvance"],false);
    }
}
