//! UI-independent GitHub transport and Actions Run collection.
//! The caller owns PAT acquisition, database writes, polling cadence, and approval policy.
//! This module never persists a credential and never advances workstreams.
use anyhow::{anyhow, Result};
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRunsResponse {
    pub workflow_runs: Vec<GithubRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubRun {
    pub id: i64,
    pub workflow_id: i64,
    pub name: String,
    pub path: Option<String>,
    pub display_title: Option<String>,
    pub event: String,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub run_number: i64,
    #[serde(default = "default_attempt")]
    pub run_attempt: i64,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: String,
    pub created_at: String,
    pub run_started_at: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub pull_requests: Vec<GithubPullRef>,
}

fn default_attempt() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubPullRef {
    pub number: i64,
}


pub fn github_client(token: &str) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );
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
        .user_agent("ci-watchtower/0.3.28")
        .timeout(Duration::from_secs(20))
        .build()?)
}

pub fn validate_repo(repo: &str) -> Result<()> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(anyhow!("Repository는 owner/name 형식이어야 합니다."));
    }
    if !owner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(anyhow!("Repository 이름에 허용되지 않은 문자가 있습니다."));
    }
    Ok(())
}


pub async fn fetch_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: String,
    label: &str,
) -> Result<T> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("{label}: HTTP {}", response.status()));
    }
    Ok(response.json().await?)
}

pub async fn github_repository_runs(client: &Client, repo: &str) -> Result<Vec<GithubRun>> {
    let url = format!("https://api.github.com/repos/{repo}/actions/runs?per_page=100");
    let data: GithubRunsResponse = fetch_json(client, url, "GitHub Actions 조회 실패").await?;
    let mut runs: HashMap<i64, GithubRun> = data.workflow_runs.into_iter().map(|run| (run.id, run)).collect();
    // Old active runs must not disappear merely because 100 newer completed runs exist.
    for status in ["queued", "in_progress", "waiting", "pending", "requested"] {
        for page in 1..=10 {
            let data: GithubRunsResponse = fetch_json(client, format!("https://api.github.com/repos/{repo}/actions/runs?status={status}&per_page=100&page={page}"), "Active Actions 조회 실패").await?;
            let count = data.workflow_runs.len();
            for run in data.workflow_runs { runs.insert(run.id, run); }
            if count < 100 { break; }
        }
    }
    Ok(runs.into_values().collect())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_untrusted_repository_path_before_api_construction() {
        for repo in ["", "only-owner", "owner/repo/other", "/repo", "owner/", 
            "owner/repo?per_page=1", "owner/../escape", "owner/repo#fragment"] {
            assert!(validate_repo(repo).is_err(), "{repo}");
        }
        for repo in ["owner/repo", "owner.name/repo_name", "owner-1/repo.2"] {
            assert!(validate_repo(repo).is_ok(), "{repo}");
        }
    }

    #[test]
    fn github_run_parsing_retains_attempt_and_pr_identity() {
        let payload=serde_json::json!({
            "id":1234,"workflow_id":7,"name":"CI","path":".github/workflows/ci.yml",
            "display_title":"CI PR 9","event":"pull_request",
            "head_branch":"feat/a","head_sha":"a".repeat(40),
            "run_number":4,"status":"completed","conclusion":"success",
            "html_url":"https://github.com/example/repo/actions/runs/1234",
            "created_at":"2026-11-01T00:00:00Z","run_started_at":null,
            "updated_at":"2026-11-01T00:02:00Z",
            "pull_requests":[{"number":9}]
        });
        let run:GithubRun=serde_json::from_value(payload).unwrap();
        assert_eq!(run.id,1234);
        assert_eq!(run.run_attempt,1);
        assert_eq!(run.pull_requests[0].number,9);
        let copied:GithubRun=serde_json::from_value(serde_json::to_value(&run).unwrap()).unwrap();
        assert_eq!(copied.head_sha,run.head_sha);
    }
}
