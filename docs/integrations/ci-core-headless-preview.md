# CI Core — Headless one-shot inspector (preview)

## Scope

The `ci-core-inspect` binary is a first **UI-independent read-only observation** path. It runs and exits without launching the CI WatchTower Tauri process or requiring the existing localhost bridge to remain alive.

It uses the **same Rust Core GitHub HTTP client / response types** as WatchTower, not a duplicate Actions collector. It fetches one exact run plus the PR before and after that run, binds it to the requested repository, PR, SHA and Attempt, and prints one JSON observation.

Example from a Windows environment with the existing GitHub PAT installed under the **same user's** Windows Credential Manager account (`ci-watchtower` / `github-pat`):

```powershell
.\ci-core-inspect.exe --repository owner/repo --pr 123 --head-sha aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --run-id 456 --attempt 1
```

The SHA, PR and Run above are placeholders. Use the real exact current PR target; do not paste PATs as arguments. `--help` requires no credentials.

## Boundary and errors

- Input is explicit: repository, exact PR number, 40-hex HEAD SHA, Run ID, Attempt. Missing/invalid flags fail before any network request.
- Windows Credential Manager obtains the existing PAT directly in memory. No token is echoed, accepted on CLI, persisted, or exposed in JSON.
- HTTP GET only: current PR before/after, exact GitHub Actions run. 3 GitHub API calls per invocation; no autonomous polling or local database reads/writes.
- Output is `contractVersion=ci-core-inspect/0.1`; **not** a replacement for WatchTower's existing /v1 HTTP bridge. `source=github_actions_direct`, current PR head, actual workflow run head, candidate head if inferable, run attempt, observed timestamp, raw CI state and conservative decision state are reported.
- If the expected PR head or run attempt changes, the decision becomes `CONFLICT`. If PR/run association cannot be proved, or an Integration workflow merely claims a candidate in the title, the decision remains `UNKNOWN`. A missing Integration pinned baseline is never verified by title parsing.
- GitHub/certificate/permission/rate limit failures return `GITHUB_UNAVAILABLE`. Credential failures return `CREDENTIAL_UNAVAILABLE`. CLI exits nonzero on either. It never turns unavailable evidence into success.
- `safeToAdvance=false`, `completeCoverage=false`, `requiredChecksVerified=false` **always**. This tool never validates required checks or operational approval, and it never sends a prompt/merges/executes a task.
- Windows CI's preview binary is a test artifact, not a signed production installer. Real user Credential Manager acceptance is still LIVE PENDING.

## Migration and next phases

1. WatchTower v1 API/poller remains unchanged. This CLI does **not** modify the installed SQLite file, registry, aliases, manual attribution or notification state.
2. Autopilot can later launch the CLI *on demand* with explicit user-authorized targets, bound invocations, caching, and independent GitHub required-check verification. A CLI observation alone must not trigger a continuation.
3. A future single-writer Core runtime must carry over existing SQLite/Track/Responsibility logic, control collection rate and avoid simultaneous WatchTower/Autopilot polling, before turning off the old app-process dependency for full observatory functionality.
4. BuildMap should consume a separately authenticated evidence gateway/export, not invoke this Windows-local process from a remote server.

**Acceptance still pending:** Real Windows PAT, real repository/PR/Run/Attempt observation and one-click Autopilot lifecycle; existing user SQLite preservation; BuildMap remote gateway. All remain separate from this preview.
