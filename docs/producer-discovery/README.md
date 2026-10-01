# Producer discovery audit — 0.3.38

Captured: 2026-10-01T06:01:38.906Z. Watchtower base: 55eebb53fb15a8cf59a918935feef9f5b1d2f1d2.

All three current default branches were read directly. Each inventory includes every workflow trigger/path filter/concurrency/job/matrix/needs/reusable workflow/timeout field, 500 recent PRs, branch evidence, 100 commits and 100 recent runs. Trigger overlaps are candidates only; no Producer workflow was changed.

## Canonical evidence

### gycha0109-beep/MyeongHa
Default branch: main; SHA: 335af1441870871837149150eb5670dc906677e7. Workflows: 47; PRs: 500.

Explicit PR keys: security, applizing, character-memory, ops, UI, topic-face, frontend-integration, face-reading, product-commerce, privacy-recovery.
Explicit retirement: product-commerce.
Responsibility: .github workflows + docs/ci/workflow-responsibility-map.json.

- Current map explicitly deprecates product-commerce; preserve its inactive historical row and do not discover it from residual markers.
- Historical UI spelling must follow existing resolver key normalization; do not invent aliases.

### gycha0109-beep/Saju
Default branch: main; SHA: f3f581a982d8ee17fbb5f678041403e7abd425da. Workflows: 165; PRs: 500.

Explicit PR keys: face-observation-engine, saju-bridge, saju-research, ci-workflow, saju, face-engine, topic-face, face-reading-binding, face-research, face-bridge, face-traditional-research, face-reading, ops.
Explicit retirement: none; inactivity alone is not retirement.
Responsibility: .github workflow inventory (no responsibility map).

- No producer responsibility map; derive project-wide CI and otherwise dynamic association without making job/domain names tracks.

### gycha0109-beep/K_beauty
Default branch: main; SHA: 1fb3f2d5698cdff6901d132cc6f5f5a9b22be3bb. Workflows: 67; PRs: 500.

Explicit PR keys: taxonomy-ai, face-research, pipeline-reliability, ops, trust, mobile, taxonomy&AI, full-report.
Explicit retirement: none; inactivity alone is not retirement.
Responsibility: .github workflows + docs/ci/workflow-responsibility-map.json + per-workflow registry.

- Current PR pipeline-reliability is absent from declared canonicalTrackKeys; preserve observed evidence separately; do not silently alias to trust.
- Historical explicit taxonomy&AI is invalid syntax and must fail closed.

## Identity and upgrade policy

The existing registered MyeongHa project owns MyeongHa and Saju. The existing Visualy project owns K_beauty. Project/repository IDs stay unchanged; no repository move or personal project seed is needed. Existing aliases remain project scoped. No new alias is inferred. The existing full-report/mobile inactivity was traced to the old registry migration batch (matching row timestamps and ledger). Current Producer declaration plus explicit PR evidence repairs only that migration-owned state while retaining IDs. Later user inactivation is preserved. UI is normalized by the existing lowercase key convention.

Project, Work Track, CI responsibility, and Workflow/Job are distinct. PR body trailers discover Work Tracks. Workflow markers never discover Track rows. MyeongHa responsibility metadata suppresses legacy technical markers. K_beauty per-workflow registry is read; observed pipeline-reliability remains distinct from trust despite the stale canonical list. No-map repositories use current workflow inventory with conservative project-wide common CI and dynamic Work Track association.

## Implementation

A generic registered-repository importer pins metadata to its current default SHA, reads the current workflow roster plus optional responsibility metadata, reads 500 PRs, recovers representative historical runs for explicit keys, and fetches current active runs beyond the recent-100 window. New tracks come only from valid, unambiguous PR trailers. Existing exact rows or aliases win, including inactive rows. A per-repository producer_discovery ledger makes version producer-discovery-v038 a one-shot transaction. Repeated runs leave later user edits intact.

The transaction retains identity/settings/keyring, manual assignments, aliases, notifications, existing runs and historical evidence. Explicit retired rows become inactive, never deleted. PR evidence backfills Work Track associations independently of project-wide primary responsibility. Invalid explicit keys fail closed; conflicting explicit keys stay unresolved. Metadata/rules and marker suppression are repository scoped. Technical domains and retired workflow markers cannot create ghost tracks.

Track history prefers its separate Work Track association over an automatic static workflow assignment; manual assignment takes precedence over both. Thus a workflow with static trust responsibility can appear in the actual pipeline-reliability PR history without also inflating the trust Work Track history. Primary responsibility and the original attribution records remain stored.

Normal polling discovers registered repositories automatically. Fresh installation stays empty. There are no MyeongHa/Saju/K_beauty runtime branches. For an existing offline DB, the optional --import-producer-snapshot CLI accepts a generic JSON snapshot, backs up the existing database before migration, and exports the actual dashboard for verification. Snapshot files are operator inputs, never bundled seed data.

The attribution view reads attempt-specific GitHub Job/Step status, conclusion, start/completion timestamps, and runner assignment. Missing timestamps stay missing; elapsed workflow time is not reported as command execution time or an inferred queue cause.

The existing DB contained 28 runs still marked active from September 23–26. Exact GitHub run queries confirmed all 28 are completed. Discovery and polling now reconcile stored active runs that fall outside recent/active listing windows by exact run ID; absence never implies completion. A bounded batch prevents unbounded polling work, and unresolved API failures preserve the stored state rather than fabricating a terminal conclusion.

## Verification

Regression tests cover fresh registries, identity/history preservation, alias ID reuse, inactive retirement, manual assignment/notifications/settings, independent project-wide responsibility and Work Track association, invalid/conflicting explicit values, repository isolation, current map formats, one-shot idempotency, and recovery of fetched historical runs. Frontend production build is required locally; Windows CI runs Rust tests and builds NSIS/MSI plus the portable EXE. Existing-user DB verification uses a separate SQLite backup before applying the same executable to the original database. See the PR and final verification report for exact current CI results.
