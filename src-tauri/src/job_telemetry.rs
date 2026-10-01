use super::*;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub(super) struct GithubJob {
    id: i64,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    runner_id: Option<i64>,
    runner_name: Option<String>,
    #[serde(default)]
    steps: Vec<GithubStep>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct GithubStep {
    name: String,
    status: String,
    conclusion: Option<String>,
    number: i64,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[derive(Deserialize)]
struct JobsResponse { jobs: Vec<GithubJob> }

#[tauri::command]
pub(super) async fn get_run_jobs(app: AppHandle, run_id: i64) -> std::result::Result<Vec<GithubJob>, String> {
    let scope = {
        let state = app.state::<AppState>();
        let conn = db(&state).map_err(|error| error.to_string())?;
        conn.query_row("SELECT mr.repo,wr.run_attempt FROM workflow_runs wr JOIN monitored_repositories mr ON mr.id=wr.repository_id WHERE wr.run_id=?", params![run_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).map_err(|error| error.to_string())?
    };
    let client = github_token().and_then(|token| github_client(&token)).map_err(|error| error.to_string())?;
    let mut jobs = Vec::new();
    for page in 1..=10 {
        let data: JobsResponse = fetch_json(&client, format!("https://api.github.com/repos/{}/actions/runs/{run_id}/attempts/{}/jobs?per_page=100&page={page}", scope.0, scope.1), "Run jobs 조회 실패").await.map_err(|error| error.to_string())?;
        let count = data.jobs.len();
        jobs.extend(data.jobs);
        if count < 100 { break; }
    }
    Ok(jobs)
}
