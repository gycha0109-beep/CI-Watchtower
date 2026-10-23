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
      "workTrackSource": "pr_marker", "workTrackActive": true, "assignmentTrackId": null,
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
- `workTrackActive=false` makes `decisionState=UNKNOWN` for a formerly linked but now-retired track.\n- `workTrackId/Key/Source` represent the work association; `assignmentTrackId/Key/Source` represent the separate primary assignment. Manual assignments override derived work association.
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


## Multi-CI / exact PR observation (additive contract in v0.3.41)

The `/v1` response envelope and original fields remain unchanged. **New optional request scope** `pr_number=<positive integer>` is supported on existing `GET /v1/runs` and `GET /v1/runs/{runId}`. `pr_number` must be combined with the stable WatchTower `repository_id` for list requests. The API still returns `completeCoverage=false`: a missing CI is **UNKNOWN / NOT OBSERVED**, not a failed or passed gate.

Each Run contains these additional fields:

| New field | Meaning |
|---|---|
| `ciKind` | `general` for `.github/workflows/ci.yml`, `integration` for `.github/workflows/ci-integration.yml`, `other` / `unknown` otherwise. It is classified from **workflow path**, never solely from the display name |
| `ciKindSource` | `workflow_path` |
| `prNumber` | A **single unambiguous** observed PR association; null when absent/conflicted |
| `observedPrNumbers` | PR numbers preserved from the GitHub Actions Run `pull_requests[]` list, not their current-head SHA |
| `prIdentitySource` | `github_pull_requests`, `integration_title`, `github_and_integration_title`, `conflict`, or `unknown` |
| `prIdentityConflict` | The Run gives several PR numbers or GitHub PR references disagree with the integration candidate; `decisionState=CONFLICT` |
| `integrationCandidateHeadSha` | Only extracted from a valid `Integration PR #<PR> at <40-hex SHA>` title **in the integration workflow file**; not inferred from general CI or unrelated workflows |
| `integrationCandidateBaseSha` | Null until WatchTower genuinely observes the **pinned integration admission baseline**; never copy the present PR base into historical runs |
| `integrationBaselineVerified` | False while pinned baseline metadata is not available |
| `headSha` (existing) | GitHub **workflow run SHA**. For a `workflow_dispatch` integration workflow this **can differ** from the tested PR candidate head. Do not interchange it with `integrationCandidateHeadSha` |

PR references arrive via the existing GitHub Actions polling payload and are persisted **additively** in the existing `workflow_runs.pr_numbers_json` column; past installations and manually assigned runs are retained. Existing rows have `[]` until directly observed again. A stored PR number is an association, **not** independent proof of the currently active PR HEAD. GitHub can expose today's PR head inside an older Run's `pull_requests[]` object; WatchTower deliberately stores only the PR **number** from it.

### Requests / response examples

```http
GET /v1/runs?repository_id=5&pr_number=2499&limit=50 HTTP/1.1
Host: 127.0.0.1:48765
Authorization: Bearer <Windows-Credential-Manager-token>
```

Illustrative subset:

```json
{
  "contractVersion": "1.0",
  "data": {
    "completeCoverage": false,
    "runs": [
      {
        "runId": 999123,
        "repositoryId": 5,
        "repository": "owner/example",
        "workflowPath": ".github/workflows/ci-integration.yml",
        "ciKind": "integration",
        "prNumber": 2499,
        "observedPrNumbers": [2499],
        "prIdentitySource": "github_and_integration_title",
        "prIdentityConflict": false,
        "headSha": "cccccccccccccccccccccccccccccccccccccccc",
        "integrationCandidateHeadSha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "integrationCandidateBaseSha": null,
        "integrationBaselineVerified": false,
        "runAttempt": 1,
        "ciState": "QUEUED",
        "decisionState": "QUEUED",
        "safeToAdvance": false
      }
    ]
  }
}
```

These example IDs/SHAs are synthetic, and fields not shown are unchanged from contract 1.0. `pr_number=2499` cannot be sent to another repository with identical PR numbers expecting the same results: WatchTower always scopes by `repository_id`.

**Scoped Job/Step view** (new minimal GET route; existing Tauri `get_run_jobs` fetch logic reused):

```http
GET /v1/runs/999123/jobs?repository_id=5&pr_number=2499&head_sha=cccccccccccccccccccccccccccccccccccccccc&attempt=1 HTTP/1.1
Host: 127.0.0.1:48765
Authorization: Bearer <Windows-Credential-Manager-token>
```

Returns `{"contractVersion":"1.0","data":{"run":{...},"jobs":[{"id":1,"name":"CI Verify","status":"completed","conclusion":"success","steps":[...]}],"completeCoverage":false}}`.
- This route is read-only but **performs an on-demand GitHub Jobs API GET** through WatchTower's existing GitHub PAT and only for an exact, fresh and unambiguous run. There is no new poller or mutation endpoint.
- Requires repository ID, PR number, exact **Run** head SHA, and latest attempt; verifies scope before contacting GitHub.
- No match = 404, wrong PR/SHA/Attempt = 409, unavailable/stale evidence or GitHub Jobs fetch = 503. Job `skipped` is returned as `skipped`, never silently converted to `success`.
- Integration full regressions may use reusable workflow Jobs whose steps are scattered across reusable workflows. This Jobs endpoint reports only what GitHub's latest-attempt Jobs API returns (up to existing pagination limit), not GitHub's full required-check evaluation.
- If `integrationCandidateHeadSha` differs from `headSha`, the **Run** SHA must still be sent to the Job endpoint, then the candidate SHA must be checked independently against the current PR HEAD by Autopilot.
- The bridge cannot verify a pinned `candidate_base` from the current stored Runs / job metadata alone. Autopilot must obtain actual admission input or authoritative integration audit for the requested PR, confirm current base branch head, and fail closed when unavailable or changed.
- A PR has 0 or more general, integration or ancillary workflows. No integration run means `NOT_OBSERVED`, **not** `FAILURE`; Autopilot decides by phase and GitHub required checks whether it is required.
- `safeToAdvance=false` remains true to its existing invariant; **never** auto-progress solely from `decisionState=SUCCESS` or a named `CI Integration Verify` job without the PR/SHA/base/required checks.

### Cancelled Run supersession (additive historical metadata, W-01)

A cancelled historical Run can be followed by another GitHub Actions Run on the **same repository, PR, workflow identity/path and tested candidate HEAD SHA**. Saju [PR #2704](https://github.com/gycha0109-beep/Saju/pull/2704) supplies an observed regression: PIE `39906779844` was cancelled and later PIE `39906837903` succeeded with the same candidate SHA.

**New additive fields on Run DTOs:**

- `historicalDisposition`: `SUPERSEDED` only for a cancelled Run when a newer, fresh, unambiguous, latest-attempt successor has been observed in the PR-scoped list. `UNRESOLVED` for a cancelled Run with no *proven* successor **in this observation**, and `NOT_APPLICABLE` for other conclusions. `UNRESOLVED` is **not** proof there is no later Run.
- `supersededByRunId`: the verified successor Run ID for a `SUPERSEDED` historical record; `null` otherwise.

These are **historical relationship annotations, not terminal CI conclusions**. Original `conclusion=cancelled`, `ciState=OTHER_TERMINAL`, immutable identity, Attempt, Job/Step history and `safeToAdvance=false` are unchanged. GitHub will continue showing the original cancelled X. There is no database rewrite or attempt to edit GitHub checks.

The candidate identity is `headSha` for normal/other PR workflows (including PIE) and **`integrationCandidateHeadSha`** for an integration dispatch workflow. A newer Run must match the same unambiguous PR number, repository, workflow ID/path and candidate SHA. The newest matching successor is considered: `SUCCESS` / `FAILURE` / `QUEUED` / `RUNNING` can supersede old cancellation **only when** its latest Attempt, CI decision and freshness are verified. A newer stale/conflicting/unverified Run never licenses fallback to an older success. Incomplete PR history cannot prove the globally latest Run.

For a **PR-scoped** `/v1/runs` list, annotations use the newest-first, bounded results returned by the query. Any newer successor to a **returned** cancelled Run appears earlier in that same prefix; this preserves the existing efficient `limit` behavior. A truncated or incomplete list never proves the absence of a newer Run. Single exact `/v1/runs/{id}` remains a raw historical lookup and does **not** claim complete supersession relationships; its cancelled Run normally reports `UNRESOLVED`. Other unscoped historical lists likewise do not claim the relation. Consumers must use a fresh PR-scoped listing and independently verify GitHub required checks/current PR HEAD before selecting an effective per-workflow result.

**Important:** WatchTower still advertises `completeCoverage=false` and does not return PR-wide GREEN, run a recovery script, send messages, or approve a merge. The downstream Autopilot must combine actual latest Check Runs, branch/ruleset requirements, integration base evidence and the existing approval gate. PIE itself does not need workflow changes for this behavior.

### Multi-repository and lifecycle semantics

- **MyeongHa:** `ci.yml` is standard PR verification and `ci-integration.yml` is separately admitted integration verification. Both can relate to the **same PR** without claiming the integration workflow is Track-owned.
- **Saju:** `ci.yml` and `production-calculation-container.yml` are separate workflows; a `[WT:saju]` name does not reassign other repository or Project-wide runs. `CI Tests (N/16)` are Jobs within a workflow, not sixteen new Work Tracks.
- The Work Track/CI Responsibility fields are unchanged. Other Track or PR successes must not be substituted for the requested Workstream.
- Historical rows pre-dating 0.3.41 with no preserved PR references remain unknown; never guess their PR solely by branch, display name or matching recent SHA. Reobserve them or independently verify through GitHub before proceeding.
- Existing Windows Credential Manager credentials, Alias, user assignments, registry, responsibility contracts and notification histories are not reset.

### Additional codes

- `409 PR_NUMBER_MISMATCH`: Exact Run belongs to another or an ambiguous PR.
- `409 ATTRIBUTION_CONFLICT`: Run has ambiguous PR identity or existing WatchTower attribution conflict (Jobs endpoint).
- `400 EXACT_RUN_SCOPE_REQUIRED`: Jobs endpoint requires repository/PR/Run SHA/Attempt.
- `503 EVIDENCE_NOT_FRESH`: Jobs endpoint refuses stale/unknown/failed polling evidence.
- `503 GITHUB_JOBS_UNAVAILABLE`: The existing GitHub Jobs API failed (including auth/rate-limit/transport).
- `400 INVALID_SCOPE`: Includes invalid `pr_number`.

### Consumer integration status

At the time of implementation, Development Autopilot's **default branch** contains planning documents but no committed Watchtower Observer / Decision Gate runtime. Its open PRs are independent, unmerged code and cannot be treated as production behavior. The consumer needs its own PR-SHA/required-check verifier and phase policy implementation; no WatchTower code may issue macros, retry decisions, SMS or PR merges.
