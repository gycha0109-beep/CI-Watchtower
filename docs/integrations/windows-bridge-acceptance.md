# P1 Windows bridge acceptance — live execution record

The **installed WatchTower app**, not CI's Windows runner, must pass this gate. The **Online/Offline** modes of [tools/Test-WatchTowerBridge.ps1](../../tools/Test-WatchTowerBridge.ps1) are read-only: no WatchTower SQLite/Registry changes, PAT access, credential export, or bridge API mutation. An explicitly selected **WindowsCanary** mode writes and deletes only a unique test credential in the isolated CI runner; do not invoke that mode as part of the user's installed-app live smoke. The script uses Windows PowerShell 5.1+ and CredReadW.

## Baseline and provenance

- Starting source baseline: WatchTower v0.3.39, `main` `0d28e4e116c5d1c41ddd099c391734bd8d059055` after PR #58.
- Actual executing installer commit/build, Windows account, date, and GitHub PR/Head SHA must be recorded in the **private** acceptance notes at execution time. Do not treat the starting baseline as the live installed build.
- The local HTTP contract is [development-autopilot.md](development-autopilot.md) v1.0. The account-specific Windows credential target is `development-autopilot-bridge-v1.ci-watchtower` (keyring 3 Windows-native maps `account.service`).
- Do **not** run a credential export or paste the token into the terminal. The script reads it from the OS and does not print or write it.
- The script never claims that HTTP responses verify all GitHub required checks, a current PR Head SHA, approval policy, or actual AI/PR execution permission.
- The Rust `keyring` writes `Entry::set_password` to Windows Generic Credentials as **UTF-16LE** (64 hex characters = 128 bytes). The credential reader decodes UTF-16LE; `-Mode SelfTest` verifies positive and malformed-token paths without OS credential access. Parsing the PowerShell script alone is not a runtime check.

## Prerequisites / data preservation

1. Identify the exact installed WatchTower release/commit and note the currently registered Project / Repository / Track IDs. Use a non-production test Repository if available; **do not delete** any existing data to construct a test.
2. Fully exit WatchTower (not just close to tray), then take a verified, restorable copy of its existing SQLite database and related WAL/SHM files with the app stopped. Record the real app data path rather than guessing it. Do not reset the DB.
3. Record an existing run's **repository ID**, **GitHub Run ID**, **current PR Head SHA**, and **latest Run Attempt** from GitHub. A SHA from a prior PR revision is not acceptable.
4. Start WatchTower under the **same Windows user** that will run the script and Autopilot. Confirm the GitHub PAT is still usable in the WatchTower UI without reading/exporting the PAT.
5. Confirm port `127.0.0.1:48765` is free before starting, or identify the owner of a collision. Do not kill unknown listeners.
6. Use Windows PowerShell with the script checked out/downloaded from the exact tested commit. Do not execute arbitrary remote script content.

## Live invocation (PowerShell, repository root)

Capture the non-secret identity baseline while WatchTower is running:

```powershell
.\tools\Test-WatchTowerBridge.ps1 -Mode Online `
  -RepositoryId 2 -RunId 12345 `
  -HeadSha aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -Attempt 1 `
  -BaselinePath "$env:TEMP\watchtower-p1-baseline.json" -WriteBaseline `
  -ReportPath "$env:TEMP\watchtower-p1-online.json"
```

**Replace all sample IDs/SHA/Attempt with current real evidence.** Optionally add `-TrackKey ops` only when the source evidence establishes that exact Work Track; Project-wide CI responsibility is a separate dimension.

While WatchTower remains running, close its window to the tray and run the same `-Mode Online` command **without** `-WriteBaseline`. Confirm that authenticated access remains available and stable IDs/keys compare. This does not test OS notification delivery.

Fully exit WatchTower and verify that a local request is no longer served:

```powershell
.\tools\Test-WatchTowerBridge.ps1 -Mode Offline -ReportPath "$env:TEMP\watchtower-p1-stopped.json"
```

Start WatchTower again as the **same OS account**, then repeat the original online command **without** `-WriteBaseline`. This validates reuse of the existing OS credential and non-destructive Project / Repository / Track identity. A separate DB-level comparison is still necessary for aliases, manual assignments, notification dedupe, and retained run history because v1 bridge does not expose all those tables.

The process returns `0` when its executable probes pass, `1` on failure, and `2` when a live Run/Repository probe is skipped. A zero exit code **does not** mean the entire P1 gate has passed; it means only the probes executed by this invocation passed.

## Additional Windows CI gates (not user's live installation)

The Windows GitHub runner now executes:
- Rust's `real_loopback_http_enforces_bridge_auth_and_shutdown`: a real TCP listener on `127.0.0.1` using an OS-selected ephemeral port with the production router and synthetic token. It checks 401/403, successful authenticated health, no-store response, and that stopping the listener rejects **new TCP connections**. A reused keep-alive connection is not a process-exit test.
- `-Mode WindowsCanary`: creates a **unique synthetic Windows Generic Credential** under the `ci-watchtower-p1-canary-*` namespace with session-only persistence, reads it using the same CredReadW/UTF-16LE code as the Online probe, and deletes it in a `finally` block. No production target/PAT/SQLite is opened by this mode. This mode is intended only for an isolated Windows CI runner, not routine use on the user's real PC.

These gates establish Windows OS API/TCP interoperability, but do **not** establish installed WatchTower lifecycle, the real user's credential continuity or Autopilot client behavior. Until an installed PC provides evidence, P1 remains **EXTERNAL VALIDATION REQUIRED**.

## What the automated live script actually checks

| Probe | Expected observation | Boundary |
|---|---|---|
| OS credential | Bridge token read from current Windows Credential Manager and format checked | Does not expose value or test another Windows account |
| Auth / locality | No credential or wrong credential -> 401, nonmatching Host / any Origin -> 403 | No browser CORS bypass assumption |
| Health | Authorized 200, contract 1.0, `Cache-Control: no-store` | Listener liveness, not GitHub freshness |
| Registry | Project / Repository / Track identities read, baseline comparison possible | Alias and dedupe tables intentionally not in API |
| Runs list | Repository-scoped 200, `completeCoverage=false`, all `safeToAdvance=false` | Empty list is never a required-check PASS |
| Exact run | Explicit Head SHA + latest Attempt match -> `targetVerified=true`; always `safeToAdvance=false` | Caller must independently prove current PR SHA and required checks |
| Contradictions | Different SHA -> 409, different Attempt -> 409, nonexistent Run -> 404 | Not a green status |
| Lifecycle | Tray preserves listener; process exit -> unavailable; restart preserves credential and identity | Credential rotation / corrupt DB must be reported |
| CI state | Current queued/running/success/failure/cancelled/timed_out etc. only when actually observed | Deterministic Rust fixtures cover unreproducible cases |

For exact Run verification, **the script requires an explicitly supplied RepositoryId + RunId + HeadSha + Attempt**. If these are missing, it records `SKIP` and returns exit 2, even when sampled runs exist. A bounded local run list never proves that the intended Autopilot PR was covered.

## Manual acceptance items (cannot be inferred from script success)

- [ ] Confirm installed v0.3.39+ EXE/MSI and backup/restore point, and record actual installer artifact/version.
- [ ] Confirm the PAT still polls successfully, a manual refresh works, and PAT and bridge credentials remain distinct.
- [ ] Confirm a real PR's current Head SHA and latest Attempt match both GitHub and bridge; queued/running/completed transitions and work association are correctly represented.
- [ ] Confirm a Project-wide workflow is not converted into track-owned CI and unrelated Work Track results are not reused.
- [ ] Confirm unassigned/conflict, stale, GitHub poll/auth error and cancelled/timed-out states fail closed. If not reproducible live, cite deterministic Rust fixture/test instead and mark the **live case unobserved**.
- [ ] Confirm an offline/network failure or port collision leads to `WATCHTOWER_UNAVAILABLE` in the **Autopilot client**; WatchTower alone cannot prove the client's decision.
- [ ] Independently compare the original and post-test DB for Project/Repository/Track IDs, aliases, manual assignments, historical runs/attempts, and notification dedupe records; allow legitimate additive poll writes. Do not compare raw SQLite file hashes during active polling.
- [ ] Confirm no token/PAT is present in console output, JSON reports, application logs, or committed artifacts.
- [ ] Record any actual Windows notification behavior separately from deterministic notification dedupe tests.

## Evidence and completion rule

Record the following **without secrets** in the project acceptance log: tested installer/head SHA, time, OS account classification (not username), reported probe results, repository + run ID + PR Head SHA + attempt, a non-secret summary of DB preservation, observed CI state transitions, and deviations. Logs/reports with private repo metadata should remain private.

**P1 exit requires** live installed Windows health/auth verification, stop/restart/credential persistence, real repository/run evidence, and preservation of pre-existing DB state. The Rust CI gate proves deterministic contract behavior; it does not replace OS/installed-app evidence.

Until the actual user's Windows environment is tested, P1 is **CODE COMPLETE · LIVE PENDING — EXTERNAL VALIDATION REQUIRED**. P2 may only address defects with a reproducible failing case; P3 client behavior belongs to Development Autopilot.
