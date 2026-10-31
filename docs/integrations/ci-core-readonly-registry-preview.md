# CI Core — Existing WatchTower registry, without Tauri (preview)

The standalone `ci-core-registry` reads an **existing** CI WatchTower SQLite file while the desktop app is stopped. It never connects to GitHub, acquires credentials, starts a poller, initializes or migrates a schema, or writes to the file. This is a separate read-only preview, not a replacement for the existing authenticated HTTP bridge.

## Windows one-shot

```powershell
.\ci-core-registry.exe --db "$env:APPDATA\com.gycha.ciwatchtower\ci-watchtower.sqlite3"
```

A caller must supply an existing **absolute** SQLite path. The command never creates missing paths. The existing Windows package uses the current user's `AppData\Roaming\com.gycha.ciwatchtower\ci-watchtower.sqlite3` as its app data store. A database locked by another application must still be read in SQLite's normal snapshot-safe manner; do not introduce a competing writer.

## Output and safety

The JSON contract is `ci-core-registry/0.1`. The data fields `projects`, `repositories`, `tracks` follow the current WatchTower bridge registry's project / repository / canonical track identities. It also supplies `observedAt`, `source=watchtower_existing_sqlite`, `readOnly=true`, `completeCoverage=false` and `safeToAdvance=false`. A repository's presence in this registry is **not** evidence of fresh CI. A nonempty historical polling error is returned only as `POLL_ERROR`, never as raw error text.

Errors return a nonzero exit status and only `INVALID_TARGET`, `DATABASE_UNAVAILABLE`, or `DATABASE_SCHEMA_UNSUPPORTED` in the JSON envelope. No credential, token, PR history, or raw GitHub response is returned. The reader does not infer approval, ownership, required-check completeness or valid merge state.

## Next dependencies

- This reader allows Autopilot to look up persisted Project / Repository / Track identities after WatchTower closes, without duplicating the SQLite writer.
- The separately built `ci-core-inspect` obtains a GitHub Actions observation for a specifically bound PR/HEAD/Run/Attempt, also without the desktop app. Neither reader promises complete CI coverage.
- Full automatic project-wide discovery, poll scheduling, notifications, manual attribution and new Run/Attempt persistence still require extracting the single-writer Core runtime from the WatchTower Tauri process. Do **not** start an independent writer alongside the existing WatchTower poller.
- BuildMap should eventually consume authenticated, minimally scoped evidence from a suitable gateway/export rather than a remote URL to local SQLite.

Windows CI builds and tests both CLI tools, publishing them in the `ci-core-headless-preview` artifact. This is an unsigned test artifact. Real user DB acceptance must be performed read-only, including count and integrity comparison.
