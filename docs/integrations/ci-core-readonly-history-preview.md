# CI Core — Read-only historical Runs, Attempts and Track affiliation

The `ci-core-history` binary is a standalone **historical evidence** reader for the existing WatchTower SQLite database. It is not a live GitHub collector, latest-effective run decision or permission to resume an Autopilot workstream.

## Usage

```powershell
.\ci-core-history.exe --db "$env:APPDATA\com.gycha.ciwatchtower\ci-watchtower.sqlite3" --repository-id 1 --limit 30
```

The repository ID must be the **local WatchTower SQLite repository ID** as shown in `ci-core-registry`, not a GitHub repository ID. Limits are 1–100 rows; the default is 30.

No database creation, migration or modification is permitted. Reads are from `workflow_runs`, `run_attempts`, `monitored_repositories`, `projects`, `run_assignments`, `run_track_associations` and `watch_tracks`, preserving manual attribution precedence over automatic association. Run ID, original attempt, latest known attempt, workflow, actual Run SHA, historical status/conclusion, Track identity and resolution state are returned. Raw internal reason messages are not returned.

The `ci-core-history/0.1` result is always `historicalOnly=true`, `completeCoverage=false`, `safeToAdvance=false`. A historical SUCCESS is **never** an indicator of current PR HEAD, required check completeness, latest attempt, fresh polling, pinned integration admission baseline, or permission to dispatch. Use the separate CI Core direct GET, independent required checks, merger/review policies and explicit user approval for any current judgement.

For safety, the CLI accepts no arbitrary SQL, no unbounded pagination, no credential input, and no write operations. The preexisting database is opened in SQLite read-only mode. An unavailable database or unsupported schema fails with a fixed sanitized error code. Historical Run data is local and not exposed to external BuildMap APIs.

## Remaining work

- Headless polling with explicit project/repository/track coordination and a **single SQLite writer**, replacing the existing Tauri poller.
- GitHub API rate limit/backoff, current PR/Run/Attempt admission, and supersession reconciliation.
- Autopilot integration of historical data as advisory only.
- User Windows acceptance and source/backup integrity comparison.

This is a test preview only; neither existing local installation nor any PR has been merged or replaced.
