use super::*;

pub(super) fn run(args: &[String]) -> Result<bool> {
    if args.first().map(String::as_str) != Some("--import-producer-snapshot") { return Ok(false); }
    if args.len() != 6 || args[2] != "--database" || args[4] != "--export-dashboard" {
        return Err(anyhow!("Usage: --import-producer-snapshot <json> --database <existing-db> --export-dashboard <json>"));
    }
    let snapshot_path = PathBuf::from(&args[1]);
    let db_path = PathBuf::from(&args[3]);
    let output_path = PathBuf::from(&args[5]);
    if !snapshot_path.is_absolute() || !db_path.is_absolute() || !output_path.is_absolute() || !db_path.is_file()
        || output_path == db_path || output_path == snapshot_path {
        return Err(anyhow!("Import requires distinct absolute paths and an existing database"));
    }
    let snapshots: Vec<producer_sync::ProducerSnapshot> = serde_json::from_slice(&std::fs::read(snapshot_path)?)?;
    let backup = db_path.with_extension("before-producer-v038.sqlite3");
    if !backup.exists() {
        let conn = Connection::open(&db_path)?;
        conn.execute("VACUUM INTO ?", params![backup.to_string_lossy()])?;
    }
    init_db(&db_path)?;
    let conn = Connection::open(&db_path)?;
    let repositories = list_repositories(&conn, false)?;
    for snapshot in &snapshots {
        let repository = repositories.iter().find(|repository| repository.repo == snapshot.repository)
            .ok_or_else(|| anyhow!("Snapshot repository is not registered: {}", snapshot.repository))?;
        producer_sync::apply(&conn, repository, snapshot)?;
    }
    let state = AppState { db_path, poll_in_flight: AtomicBool::new(false) };
    let dashboard = build_dashboard_with_token_status(&state, false)?;
    std::fs::write(output_path, serde_json::to_vec_pretty(&dashboard)?)?;
    Ok(true)
}
