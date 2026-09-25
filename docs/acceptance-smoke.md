# Real-use Acceptance Smoke

This smoke gate validates the smallest complete WatchTower user journey without expanding product scope.

## Automated gate

The Rust test `fresh_install_real_use_acceptance_smoke` starts from the production database initializer and verifies:

1. a fresh database contains no pre-seeded user Project, Repository, Track, or rules;
2. Project registration succeeds through the same validation/write path used by the Tauri command;
3. Repository registration succeeds and remains Project-scoped;
4. Track registration succeeds with its long-CI threshold;
5. GitHub-style workflow run payloads are persisted through `upsert_run`;
6. a known explicit `[WT:<track-key>]` marker resolves to the registered Track;
7. an unknown explicit Track key fails closed into Unassigned;
8. failed and long-running CI notification events are deduplicated by run + attempt + event;
9. a synced repository responsibility contract with a missing WatchTower declaration appears as Responsibility Drift;
10. the production dashboard builder exposes Running, Queued, Track health, Unassigned, source health, and Drift consistently.

This test is deterministic and requires no PAT, network access, desktop notification permission, or private repository.

## Live desktop boundary

The automated gate intentionally does not fake operating-system notification delivery or GitHub authentication. A release candidate is live-smoke complete when a desktop run confirms:

1. save a valid GitHub PAT;
2. register a Project, Repository, and Track;
3. trigger or observe a real GitHub Actions run for that Repository;
4. press refresh/poll and confirm the run appears once with the correct run attempt;
5. confirm explicit Track evidence is attributed to the intended Track;
6. confirm an unknown/conflicting explicit key appears in CI Issues instead of being guessed;
7. observe one completed failure notification and one long-CI notification when those states occur;
8. confirm repeated polling does not duplicate the same notification event;
9. when `docs/ci/workflow-responsibility-map.json` exists, confirm source health and any real drift are visible in CI Issues.

## Exit rule

PASS requires the automated gate to be green in both PR CI and merged-main CI. The live desktop boundary is recorded separately because CI cannot prove native notification presentation or a user's local credential store. Any failure found there should produce a narrow bug fix, not a new governance subsystem.
