//! Repository discovery contains no project-specific registry or runtime branches.
use super::*;
use serde_json::Value;

const SYNC_VERSION: &str = "producer-discovery-v038";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProducerPull {
    pub number: i64,
    pub keys: Vec<String>,
    pub head_sha: String,
    pub merge_sha: Option<String>,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProducerWorkflow {
    pub path: String,
    pub name: String,
    pub binding: String,
    pub responsibility: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProducerSnapshot {
    pub repository: String,
    pub default_branch: String,
    pub sha: String,
    pub pulls: Vec<ProducerPull>,
    pub retired_keys: Vec<String>,
    #[serde(default)]
    pub declared_keys: Vec<String>,
    pub workflows: Vec<ProducerWorkflow>,
    #[serde(default)]
    pub historical_runs: Vec<GithubRun>,
    #[serde(default)]
    pub current_runs: Vec<GithubRun>,
}

pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS producer_discovery (
      repository_id INTEGER PRIMARY KEY REFERENCES monitored_repositories(id) ON DELETE CASCADE,
      version TEXT NOT NULL, default_branch TEXT NOT NULL, head_sha TEXT NOT NULL,
      snapshot_json TEXT NOT NULL, applied_at TEXT NOT NULL
    );")?;
    Ok(())
}

pub(super) fn applied(conn: &Connection, repository_id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM producer_discovery WHERE repository_id=? AND version=?)",
        params![repository_id, SYNC_VERSION], |row| row.get::<_, i64>(0))? != 0)
}

pub(super) fn missing_active_run_ids(conn: &Connection, repository_id: i64, runs: &[GithubRun]) -> Result<Vec<i64>> {
    let seen: HashSet<i64> = runs.iter().map(|run| run.id).collect();
    let mut stmt = conn.prepare("SELECT run_id FROM workflow_runs WHERE repository_id=? AND status IN ('queued','in_progress','waiting','pending','requested') ORDER BY last_seen_at ASC")?;
    let rows = stmt.query_map(params![repository_id], |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().filter(|id| !seen.contains(id)).take(100).collect())
}

pub(super) async fn reconcile_active_runs(client: &Client, repository: &MonitoredRepository, state: &AppState, runs: &mut Vec<GithubRun>) -> Result<()> {
    let missing = { let conn = db(state)?; missing_active_run_ids(&conn, repository.id, runs)? };
    // Absence from recent/active lists is not a terminal status. Ask GitHub about the exact run.
    for id in missing {
        let run: GithubRun = fetch_json(client, format!("https://api.github.com/repos/{}/actions/runs/{id}", repository.repo), "Stored active Run reconciliation").await?;
        runs.push(run);
    }
    Ok(())
}

pub(super) fn trailer_keys(text: &str) -> Vec<String> {
    // Retain invalid explicit values as evidence: they must not fall through to heuristics.
    text.lines().filter_map(|line| line.trim().strip_prefix("Watchtower-Track:")
        .map(|value| value.trim().to_lowercase())).collect()
}

pub(super) fn entry_binding(entry: &Value, name: &str) -> (String, String) {
    let responsibility = entry["responsibility"].as_str().or_else(|| entry["primaryResponsibility"].as_str()).unwrap_or("workflow-verification").to_string();
    let binding = entry["watchtowerTrackBinding"].as_str().map(str::to_string).unwrap_or_else(|| {
        if responsibility.starts_with("project-wide-") || (entry.is_null() && matches!(name, "CI" | "Fast Check" | "Governance")) {
            "unassigned-by-design".into()
        } else { "dynamic-by-run".into() }
    });
    (responsibility, binding)
}

async fn content(client: &Client, repo: &str, path: &str, sha: &str) -> Result<Option<Value>> {
    if path.starts_with('/') || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(anyhow!("Invalid producer metadata path"));
    }
    let response = client.get(format!("https://api.github.com/repos/{repo}/contents/{path}?ref={sha}"))
        .header(header::ACCEPT, "application/vnd.github.raw+json").send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND { return Ok(None); }
    if !response.status().is_success() { return Err(anyhow!("Producer metadata HTTP {}", response.status())); }
    Ok(Some(serde_json::from_str(&response.text().await?)?))
}

pub(super) async fn discover(client: &Client, repo: &str) -> Result<ProducerSnapshot> {
    validate_repo(repo)?;
    let metadata: Value = fetch_json(client, format!("https://api.github.com/repos/{repo}"), "Repository discovery").await?;
    let branch = metadata["default_branch"].as_str().ok_or_else(|| anyhow!("Missing default branch"))?;
    let head: Value = fetch_json(client, format!("https://api.github.com/repos/{repo}/commits/{}", urlencoding::encode(branch)), "Default branch discovery").await?;
    let sha = head["sha"].as_str().ok_or_else(|| anyhow!("Missing default head"))?;
    let roster: Vec<Value> = fetch_json(client, format!("https://api.github.com/repos/{repo}/contents/.github/workflows?ref={sha}"), "Default branch workflows").await?;
    let mut names = HashMap::new();
    for page in 1..=10 {
        let data: GithubWorkflowsResponse = fetch_json(client, format!("https://api.github.com/repos/{repo}/actions/workflows?per_page=100&page={page}"), "Workflow inventory").await?;
        let count = data.workflows.len();
        names.extend(data.workflows.into_iter().map(|workflow| (workflow.path, workflow.name)));
        if count < 100 { break; }
    }
    let map = content(client, repo, RESPONSIBILITY_MAP_PATH, sha).await?.unwrap_or(Value::Null);
    let retired_keys: Vec<String> = map["authority"]["deprecatedWorkTrackKeys"].as_array()
        .map(|values| values.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let declared_keys: Vec<String> = map["watchtowerProject"]["canonicalTrackKeys"].as_array()
        .map(|values| values.iter().filter_map(|v|v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut workflows = Vec::new();
    for file in roster {
        let path = file["path"].as_str().ok_or_else(|| anyhow!("Missing workflow path"))?;
        if !(path.ends_with(".yml") || path.ends_with(".yaml")) { continue; }
        let filename = path.rsplit('/').next().unwrap_or(path);
        let name = names.get(path).cloned().unwrap_or_else(|| filename.to_string());
        let (entry, source) = if let Some(directory) = map["workflowRegistry"]["directory"].as_str() {
            let source = format!("{directory}/{filename}.json");
            let entry = content(client, repo, &source, sha).await?.ok_or_else(|| anyhow!("Missing registered workflow metadata: {source}"))?;
            if entry["workflow"].as_str() != Some(filename) { return Err(anyhow!("Workflow registry path mismatch: {filename}")); }
            (entry, source)
        } else { (map["workflows"][filename].clone(), RESPONSIBILITY_MAP_PATH.to_string()) };
        let (responsibility, binding) = entry_binding(&entry, &name);
        // A current map with technical/PR attribution never converts a static legacy marker to a Work Track.
        workflows.push(ProducerWorkflow { path: path.to_string(), name, binding, responsibility,
            source: if entry.is_null() { ".github/workflows (default branch inventory)".into() } else { source } });
    }
    let mut pulls = Vec::new();
    for page in 1..=5 {
        let batch: Vec<Value> = fetch_json(client, format!("https://api.github.com/repos/{repo}/pulls?state=all&sort=created&direction=desc&per_page=100&page={page}"), "PR Track discovery").await?;
        let count = batch.len();
        for pr in batch {
            pulls.push(ProducerPull { number: pr["number"].as_i64().ok_or_else(|| anyhow!("Missing PR number"))?,
                keys: trailer_keys(pr["body"].as_str().unwrap_or_default()), head_sha: pr["head"]["sha"].as_str().unwrap_or_default().into(),
                merge_sha: pr["merge_commit_sha"].as_str().map(str::to_string), branch: pr["head"]["ref"].as_str().unwrap_or_default().into() });
        }
        if count < 100 { break; }
    }
    let mut historical_runs = HashMap::new();
    let mut seen_keys = HashSet::new();
    for pr in &pulls {
        let distinct: HashSet<_> = pr.keys.iter().collect();
        if distinct.len() != 1 { continue; }
        let key = &pr.keys[0];
        if validate_track_key(key).is_err() || retired_keys.contains(key) || !seen_keys.insert(key.clone()) { continue; }
        for candidate in [Some(pr.head_sha.as_str()), pr.merge_sha.as_deref()].into_iter().flatten() {
            let data: GithubRunsResponse = fetch_json(client, format!("https://api.github.com/repos/{repo}/actions/runs?head_sha={candidate}&per_page=100"), "Track run recovery").await?;
            for run in data.workflow_runs { historical_runs.insert(run.id, run); }
        }
    }
    let current_runs = github_repository_runs(client, repo).await?;
    Ok(ProducerSnapshot { repository: repo.into(), default_branch: branch.into(), sha: sha.into(), pulls, retired_keys, declared_keys, workflows, historical_runs: historical_runs.into_values().collect(), current_runs })
}

pub(super) fn contracts(snapshot: &ProducerSnapshot) -> Vec<RepositoryResponsibilityContract> {
    snapshot.workflows.iter().map(|workflow| {
        let (binding_kind, track_key) = normalize_repository_binding(&workflow.binding);
        RepositoryResponsibilityContract { workflow_path: workflow.path.clone(), workflow_name: workflow.name.clone(),
            binding_kind, track_key, source_binding: workflow.binding.clone(), source_path: workflow.source.clone() }
    }).collect()
}

pub(super) fn apply(conn: &Connection, repository: &MonitoredRepository, snapshot: &ProducerSnapshot) -> Result<bool> {
    if snapshot.repository != repository.repo { return Err(anyhow!("Producer snapshot repository mismatch")); }
    if applied(conn, repository.id)? { return Ok(false); }
    let actual: i64 = conn.query_row("SELECT project_id FROM monitored_repositories WHERE id=? AND repo=?",
        params![repository.id, repository.repo], |row| row.get(0))?;
    if actual != repository.project_id { return Err(anyhow!("Producer project scope changed")); }
    let now = Utc::now().to_rfc3339();
    let tx = conn.unchecked_transaction()?;
    for run in &snapshot.historical_runs { upsert_run(&tx, repository.id, run, &now)?; }
    for run in &snapshot.current_runs { upsert_run(&tx, repository.id, run, &now)?; }
    let running = snapshot.current_runs.iter().filter(|run| run.status == "in_progress").count() as i64;
    let queued = snapshot.current_runs.iter().filter(|run| matches!(run.status.as_str(), "queued" | "waiting" | "pending" | "requested")).count() as i64;
    tx.execute("UPDATE monitored_repositories SET running_count=?,queued_count=?,last_polled_at=?,last_successful_poll_at=?,last_error=NULL WHERE id=?", params![running,queued,now,now,repository.id])?;
    let keys: HashSet<String> = snapshot.pulls.iter().filter(|pr| {
        let keys: HashSet<_> = pr.keys.iter().collect();
        keys.len() == 1 && keys.iter().all(|key| validate_track_key(key).is_ok())
    }).flat_map(|pr| pr.keys.clone()).filter(|key| !snapshot.retired_keys.contains(key)).collect();
    legacy_compat::restore_producer_confirmed_inactive_tracks(&tx,repository.project_id,&keys,&snapshot.declared_keys,&now)?;
    for key in keys {
        // Existing rows, inactive rows and user aliases all win; no upsert overwrites user edits.
        let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM watch_tracks WHERE project_id=? AND track_key=?) OR EXISTS(SELECT 1 FROM track_aliases WHERE project_id=? AND alias_key=?)",
            params![repository.project_id, key, repository.project_id, key], |row| Ok(row.get::<_, i64>(0)? != 0))?;
        if !existing {
            tx.execute("INSERT INTO watch_tracks(project_id,name,track_key,long_ci_minutes,active,created_at,updated_at) VALUES(?,?,?,8,1,?,?)",
                params![repository.project_id, key, key, now, now])?;
        }
    }
    for key in &snapshot.retired_keys {
        tx.execute("UPDATE watch_tracks SET active=0,updated_at=? WHERE project_id=? AND track_key=?", params![now, repository.project_id, key])?;
    }
    let contracts = contracts(snapshot);
    replace_repository_responsibility_contracts_inner(&tx, repository.id, &contracts, &now)?;
    for workflow in &snapshot.workflows {
        if workflow.binding == "unassigned-by-design" {
            tx.execute("INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at) VALUES(?,?,?,1,?)",
                params![repository.project_id, repository.id, workflow.name, now])?;
        } else if workflow.binding == "dynamic-by-run" {
            tx.execute("INSERT INTO dynamic_workflow_rules(project_id,repository_id,workflow_name,active,protected,created_at) SELECT ?,?,?,1,0,? WHERE NOT EXISTS(SELECT 1 FROM dynamic_workflow_rules WHERE project_id=? AND repository_id=? AND workflow_name=?)",
                params![repository.project_id, repository.id, workflow.name, now, repository.project_id, repository.id, workflow.name])?;
        }
    }
    // This snapshot also governs historical run-name filtering and is committed with all associations.
    tx.execute("INSERT INTO producer_discovery(repository_id,version,default_branch,head_sha,snapshot_json,applied_at) VALUES(?,?,?,?,?,?)",
        params![repository.id, SYNC_VERSION, snapshot.default_branch, snapshot.sha, serde_json::to_string(snapshot)?, now])?;
    backfill(&tx, repository, snapshot, &now)?;
    update_repository_responsibility_source(&tx, repository.id, "synced", &now, None)?;
    tx.commit()?;
    Ok(true)
}

pub(super) fn filter_evidence(snapshot: &ProducerSnapshot, path: Option<&str>, name: &str, evidence: &mut Vec<Evidence>) -> bool {
    let original_len = evidence.len();
    let workflow = snapshot.workflows.iter().find(|workflow| Some(workflow.path.as_str()) == path)
        .or_else(|| snapshot.workflows.iter().find(|workflow| workflow.name == name));
    let metadata_binding = workflow.is_some_and(|workflow| workflow.source != ".github/workflows (default branch inventory)");
    evidence.retain(|item| !(item.signal_type == "run_name" && (snapshot.retired_keys.contains(&item.track_key)
        || (metadata_binding && workflow.is_some_and(|workflow| !workflow.binding.starts_with("static:"))))));
    evidence.len() != original_len
}

pub(super) fn load_snapshot(conn: &Connection, repository_id: i64) -> Result<Option<ProducerSnapshot>> {
    let raw: Option<String> = conn.query_row("SELECT snapshot_json FROM producer_discovery WHERE repository_id=?", params![repository_id], |row| row.get(0)).optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into)).transpose()
}

fn pull_evidence(snapshot: &ProducerSnapshot, sha: &str, branch: Option<&str>) -> Vec<Evidence> {
    let exact: Vec<_> = snapshot.pulls.iter().filter(|pr| pr.head_sha == sha || pr.merge_sha.as_deref() == Some(sha)).collect();
    let pulls = if exact.is_empty() { snapshot.pulls.iter().filter(|pr| branch == Some(pr.branch.as_str())).collect() } else { exact };
    pulls.iter().flat_map(|pr| pr.keys.iter().map(|key| Evidence { track_key: key.clone(), signal_type: "pr_marker".into(), score: 98, value: format!("PR #{} (producer discovery)", pr.number) })).collect()
}

fn backfill(conn: &Connection, repository: &MonitoredRepository, snapshot: &ProducerSnapshot, now: &str) -> Result<()> {
    let tracks: Vec<_> = list_tracks(conn, true)?.into_iter().filter(|track| track.project_id == repository.project_id).collect();
    let mut aliases = load_project_aliases(conn, repository.project_id)?;
    for key in &snapshot.retired_keys { aliases.remove(key); }
    let rules = list_project_workflow_rules(conn)?;
    let runs = {
        let mut stmt = conn.prepare("SELECT run_id,head_sha,head_branch,workflow_path,workflow_name,resolution_status FROM workflow_runs WHERE repository_id=? AND ignored=0 AND NOT EXISTS(SELECT 1 FROM run_assignments WHERE run_id=workflow_runs.run_id AND manual=1)")?;
        let rows = stmt.query_map(params![repository.id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (run_id, sha, branch, path, name, status) in runs {
        let mut evidence = {
            let mut stmt = conn.prepare("SELECT track_key,signal_type,score,value FROM run_evidence WHERE run_id=?")?;
            let rows = stmt.query_map(params![run_id], |row| Ok(Evidence { track_key: row.get(0)?, signal_type: row.get(1)?, score: row.get(2)?, value: row.get(3)? }))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let discovered = pull_evidence(snapshot, &sha, branch.as_deref());
        let recovered_pr = !discovered.is_empty();
        if recovered_pr { evidence.retain(|item| item.signal_type != "pr_marker"); evidence.extend(discovered); }
        let discarded_marker = filter_evidence(snapshot, path.as_deref(), &name, &mut evidence);
        let (association, explicit) = work_track_association(&tracks, &aliases, &evidence);
        persist_work_track_association(conn, run_id, association, explicit || discarded_marker, now)?;
        let project = project_rule_matches(&rules, repository.project_id, repository.id, &name);
        if project || recovered_pr || discarded_marker || matches!(status.as_str(), "unassigned" | "conflict") {
            let resolution = if project { Resolution { status: "project".into(), track_id: None, confidence: Some(100), source: Some("project_workflow".into()), reason: Some(format!("프로젝트 공용 CI: {name}")), evidence } }
                else { resolve_evidence(&tracks, &aliases, evidence) };
            persist_resolution_with_trigger(conn, run_id, &resolution, now, Some("producer_discovery"))?;
        } else {
            // Append newly recovered PR evidence without deleting historical attribution evidence.
            for item in evidence.iter().filter(|item| item.signal_type == "pr_marker") {
                conn.execute("INSERT INTO run_evidence(run_id,track_key,signal_type,score,value,created_at) SELECT ?,?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM run_evidence WHERE run_id=? AND track_key=? AND signal_type=? AND value=?)",
                    params![run_id,item.track_key,item.signal_type,item.score,item.value,now,run_id,item.track_key,item.signal_type,item.value])?;
            }
        }
    }
    Ok(())
}
