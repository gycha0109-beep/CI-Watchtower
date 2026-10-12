# Development Autopilot read-only bridge (contract 1.0)

The WatchTower desktop process owns polling, SQLite, Project/Track registry, and GitHub attribution. Development Autopilot consumes observations; it **must** independently verify exact PR head SHA, required checks, review/merge constraints, and any approval policy. No API here authorizes a merge, code execution, or phase advancement.

## Transport, runtime, configuration

- **Transport:** HTTP GET on `http://127.0.0.1:48765/v1`. The server is started by the existing WatchTower Tauri process after its normal database initialization; there is **no sidecar process, separate poll loop, or remote listener**.
- **Authentication:** Bearer token stored only in the current Windows user's Credential Manager via the `keyring` crate. Credential target: service `ci-watchtower`, account `development-autopilot-bridge-v1`. WatchTower generates a random 256-bit token on first startup, reuses it across restarts, and never stores it in SQLite, UI, source control, HTTP responses, or logs. The GitHub PAT uses a **different** credential account and is never exposed.
- **Autopilot client setup:** Retrieve the same Windows Credential Manager entry using the Windows credential API or an OS credential-library such as `keyring`, then send `Authorization: Bearer <secret>`. Do not copy the credential into Chrome extension storage or repository documentation. Start WatchTower before requesting data. There is no token-export HTTP endpoint.
- **Security boundary:** Listen only on `127.0.0.1`. Reject a nonmatching `Host`, all `Origin` headers, and requests without the token; no CORS allowance. Local HTTPS is not used because no traffic leaves loopback. The interface is **same-user local-process authenticated**, not a general multi-user security perimeter: other programs running as that Windows user may be able to retrieve its Windows Credential Manager credential. Local malware with access to that account is out of scope.
- **Port collision:** If port 48765 is occupied, the bridge is unavailable, and WatchTower continues its normal UI/poller. Check the application's standard error/diagnostics to identify binding problems.
- **Restart/stop:** Closing WatchTower's window to the tray leaves the process (and bridge) running. Fully exiting WatchTower closes the listener. Connection refused/timeout is **WATCHTOWER_UNAVAILABLE**, never successful CI.
- **Read-only DB:** Every request opens a SQLite `SQLITE_OPEN_READ_ONLY` connection to the existing file. No externally callable update/PR/notification, migration or poll action is registered. Existing database/registry/aliases/notification dedupe remain untouched.
- **Freshness:** The response is derived from last successful repository poll, the most recent repository error, the individual Run `last_seen_at`, and the latest known attempt. Allowed age is `clamp(3 × max(activePollSeconds, idlePollSeconds), 180s, 3600s)`. Both repository success and individual Run observation must meet this limit. A latest-attempt discrepancy is `STALE`. Poll errors take precedence over cached GREEN.
- **Operational limitation:** WatchTower requests up to the most recent 100 runs plus all active runs; older completed runs can drop out. The API returns a bounded historical sample, not the complete required-check set. For a completed PR, use the separate authoritative GitHub verifier.

## Endpoints (version 1.0)

All calls must have `Host: 127.0.0.1:48765` (HTTP clients set this automatically), `Authorization: Bearer <token>`, and must **not** send an `Origin` header. The bridge returns JSON only with `Cache-Control: no-store`.

| Endpoint | Result | Scope |
|---|---|---|
| `GET /v1/health` | Bridge connection/contract version | authenticated |
| `GET /v1/registry?project_id=10` | Minimal Project/Repository/Track identity and poll health | optional project ID or repository ID |
| `GET /v1/runs?repository_id=20&track_key=character-memory&head_sha=<sha>&limit=30` | Up to 100 locally observed runs, newest first | **repository ID required**; optional project ID, canonical track key, 40-hex head SHA, limit 1..100 |
| `GET /v1/runs/123?repository_id=20&head_sha=<sha>&attempt=2` | Exact stored run and observed latest attempt | run ID mandatory; repository ID optional; **SHA and attempt strongly required before consuming evidence** |

Names use `snake_case` for query parameters and `camelCase` for JSON fields.

### Registry response

```json
{
  "contractVersion": "1.0",
  "data": {
    "projects": [{"id": 1, "key": "example", "name": "Example", "active": true}],
    "repositories": [{
      "id": 2, "projectId": 1, "name": "owner/repo", "enabled": true,
      "lastPollAttemptAt": "2026-10-13T00:00:00Z",
      "lastSuccessfulPollAt": "2026-10-13T00:00:00Z", "pollErrorCode": null
    }],
    "tracks": [{"id": 3, "projectId": 1, "canonicalTrackKey": "example-work",
                "name": "Example Work", "active": true}]
  }
}
```

Values above are illustrative, not actual local observations.

### Run response

```json
{
  "contractVersion": "1.0",
  "data": {
    "run": {
      "runId": 123, "projectId": 1, "projectKey": "example",
      "repositoryId": 2, "repository": "owner/repo", "repositoryEnabled": true,
      "workflowId": 44, "workflowName": "CI", "workflowPath": ".github/workflows/ci.yml",
      "headSha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "headBranch": "feat/example", "runAttempt": 2, "latestKnownAttempt": 2,
      "status": "completed", "conclusion": "success",
      "ciState": "SUCCESS", "decisionState": "SUCCESS", "evidenceState": "FRESH",
      "responsibilityKind": "project-wide", "technicalResponsibility": "project-wide-ci",
      "resolutionStatus": "project", "workTrackId": 3, "workTrackKey": "example-work",
      "workTrackSource": "pr_marker", "assignmentTrackId": null,
      "assignmentTrackKey": null, "assignmentSource": null,
      "attributionConflict": false,
      "createdAt": "2026-10-13T00:00:00Z",
      "updatedAt": "2026-10-13T00:01:00Z",
      "lastSeenAt": "2026-10-13T00:01:00Z",
      "lastPollAttemptAt": "2026-10-13T00:01:00Z",
      "lastSuccessfulPollAt": "2026-10-13T00:01:00Z",
      "pollErrorCode": null, "observedAt": "2026-10-13T00:01:01Z",
      "freshnessLimitSeconds": 270, "targetVerified": true, "safeToAdvance": false
    }
  }
}
```

- `ciState`: `QUEUED`, `RUNNING`, `SUCCESS`, `FAILURE`, `OTHER_TERMINAL`, `UNKNOWN`. The raw GitHub `status` and `conclusion` are also present (cancelled/timed_out must not be treated as success).
- `evidenceState`: `FRESH`, `STALE`, `UNKNOWN`, `POLL_ERROR`. A repository GitHub authentication/rate-limit error is exposed as `pollErrorCode` without raw API error text or tokens.
- `decisionState`: `CONFLICT` when primary attribution is conflicted; otherwise `STALE`, `POLL_ERROR` or `UNKNOWN` override raw `ciState`. Only when fresh/non-conflicting does it agree with `ciState`.
- `responsibilityKind`: `project-wide`, `dynamic`, `track-owned`, `unknown`. Sourced from WatchTower rules and producer snapshot. A work-track link **does not** transform a Project-wide CI into a track-owned workflow.
- `workTrackId/Key/Source` represent the work association; `assignmentTrackId/Key/Source` represent the separate primary assignment. Manual assignments override derived work association.
- `targetVerified` is true only if the caller provided matching exact SHA **and** run attempt and the latest stored attempt agrees. It does *not* prove that this is the latest PR head on GitHub.
- `safeToAdvance` is **always false by contract**: the WatchTower bridge alone cannot prove required-check coverage, PR head, merge state or human approvals. Autopilot must compose independent GitHub verification and local policy.
- `GET /v1/runs` wraps records in `data.runs` and sets `data.completeCoverage=false`. Empty list ≠ all checks passed; out-of-scope runs must not satisfy this Work Track's requirements.
- A successfully completed historical run without recent observation becomes `STALE`, even if the repository has continued polling other runs. This is an intentional fail-closed constraint.

### Status and errors

Responses use the envelope `{"contractVersion":"1.0","error":{"code":"..."}}`.

| HTTP | Code | Meaning/action |
|---|---|---|
| 400 | `INVALID_SCOPE`, `INVALID_RUN_ID`, `REPOSITORY_SCOPE_REQUIRED` | Correct inputs; no automatic progress |
| 401 | `UNAUTHORIZED` | Retrieve current OS-stored bridge credential |
| 403 | `LOCAL_CLIENT_ONLY` | Host mismatch or browser Origin; reject |
| 404 | `RUN_NOT_FOUND` | No matching stored run; not an implicit success |
| 409 | `HEAD_SHA_MISMATCH`, `RUN_ATTEMPT_MISMATCH` | Reject old SHA / old successful attempt; reverify GitHub |
| 503 | `DATABASE_UNAVAILABLE` | WatchTower DB unavailable; stop automated advancement |
| Network | `WATCHTOWER_UNAVAILABLE` (client-side) | WatchTower stopped / port not bound / network issue |
| Data | `STALE`, `UNKNOWN`, `POLL_ERROR`, `CONFLICT` | Return evidence but **block** automatic progress |

## Compatibility and limitations

- API major version `/v1` is stable for 1.x. New optional JSON fields are permitted; renamed/removed fields or semantic changes require `/v2`.
- This is a read-only observational bridge, not an execution/merge API. Existing Tauri commands remain private and unchanged.
- No push events or sync-to-Autopilot: poll the bridge at bounded intervals and use timestamps. No new GitHub requests are performed by these endpoints.
- Windows Credential Manager credential is per Windows account; running Autopilot under a different user requires explicit operator configuration. **Do not use the GitHub PAT** as a bridge token.
- Live Windows smoke: launch v0.3.39, verify `/v1/health` with the credential and 401 without it; stop the process and verify connection refused; restart, ensure identical credential and existing registry/CI history intact. Actual Windows OS-user credential/port availability is an external environment check, not demonstrated by Rust unit tests.
- Prefer a run query constrained to repository + expected head SHA + exact attempt, then obtain the independently verified PR context/required checks from GitHub. A single green Run, including Project-wide CI, never releases an Autopilot workstream by itself.
