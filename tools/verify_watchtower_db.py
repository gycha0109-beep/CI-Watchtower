#!/usr/bin/env python3
"""Read-only, fail-closed preservation evidence for an EXISTING WatchTower SQLite DB.

Run only when WatchTower is fully exited, with a verified separate DB backup.
This is not a backup or restore tool. Proof files contain salted row fingerprints;
keep them private and never commit them.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import secrets
import sqlite3
import sys

PROOF_VERSION = 1
# Only historical or user-owned fields: poll updates may change workflow status,
# latest attempt, timestamps, or add rows without invalidating preservation.
# Queries do not read PATs, credentials, poll errors, run descriptions, or notes.
PROTECTED = {
    "projects": ("id", "project_key", "name", "active", "created_at"),
    "monitored_repositories": ("id", "project_id", "repo", "enabled", "created_at"),
    "watch_tracks": ("id", "project_id", "track_key", "name", "active",
                     "legacy_track_id", "long_ci_minutes", "created_at"),
    "track_aliases": ("id", "project_id", "alias_key", "track_id", "active", "created_at"),
    "project_workflow_rules": ("id", "project_id", "repository_id",
                               "workflow_name", "active", "created_at"),
    "dynamic_workflow_rules": ("id", "project_id", "repository_id",
                               "workflow_name", "active", "protected", "created_at"),
    "run_assignments": ("run_id", "track_id", "confidence", "source",
                        "reason", "manual", "assigned_at"),
    "workflow_runs": ("run_id", "repository_id", "workflow_id", "head_sha", "created_at"),
    "run_attempts": ("run_id", "run_attempt"),
    "notifications_v2": ("track_id", "run_id", "run_attempt", "event_type", "notified_at"),
    "responsibility_escalation_deliveries": ("review_key", "fingerprint", "event_type"),
    "schema_migrations": ("migration_key", "applied_at"),
}
FILTERS = {"run_assignments": " WHERE manual=1"}


def collect(db_path: Path, salt: str) -> dict:
    if not db_path.is_file():
        raise ValueError("DB file does not exist. Never create an empty DB for acceptance.")
    # mode=ro fails rather than creating an absent DB. No user DB write commands.
    conn = sqlite3.connect(db_path.resolve().as_uri() + "?mode=ro", uri=True, timeout=5)
    try:
        conn.execute("PRAGMA query_only=ON")
        conn.execute("BEGIN")
        if conn.execute("PRAGMA quick_check").fetchone()[0] != "ok":
            raise ValueError("SQLite quick_check failed")
        existing = {row[0] for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")}
        missing = sorted(set(PROTECTED) - existing)
        if missing:
            raise ValueError("Required tables missing: " + ", ".join(missing))
        result = {}
        key = bytes.fromhex(salt)
        for table, fields in PROTECTED.items():
            actual = {row[1] for row in conn.execute(f"PRAGMA table_info({table})")}
            missing_columns = sorted(set(fields) - actual)
            if missing_columns:
                raise ValueError(f"Required columns missing in {table}: {', '.join(missing_columns)}")
            query = f"SELECT {', '.join(fields)} FROM {table}" + FILTERS.get(table, "")
            hashes = []
            for row in conn.execute(query):
                raw = json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
                hashes.append(hashlib.blake2b(raw, key=key, digest_size=32).hexdigest())
            result[table] = sorted(hashes)
        conn.execute("COMMIT")
        return result
    finally:
        conn.close()


def capture(db_path: Path, proof_path: Path) -> None:
    salt = secrets.token_hex(32)
    tables = collect(db_path, salt)
    payload = {"format": PROOF_VERSION, "salt": salt, "tables": tables}
    # Exclusive creation ensures an older baseline cannot be accidentally overwritten.
    with proof_path.open("x", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, ensure_ascii=True)
        f.write("\n")
    print("PASS: read-only SQLite baseline created (private row hashes only).")
    print("Preserved categories: " + ", ".join(f"{k}={len(v)}" for k, v in tables.items()))


def verify(db_path: Path, proof_path: Path) -> bool:
    if not proof_path.is_file():
        raise ValueError("Baseline not found. Capture it BEFORE the app restart.")
    payload = json.loads(proof_path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or payload.get("format") != PROOF_VERSION:
        raise ValueError("Unknown preservation proof format")
    salt = payload.get("salt")
    expected = payload.get("tables")
    if not isinstance(salt, str) or len(salt) != 64:
        raise ValueError("Malformed proof salt")
    if not isinstance(expected, dict) or set(expected) != set(PROTECTED):
        raise ValueError("Missing or unexpected proof tables")
    current = collect(db_path, salt)
    passed = True
    for table in PROTECTED:
        old = expected[table]
        if not isinstance(old, list) or not all(isinstance(v, str) and len(v) == 64 for v in old):
            raise ValueError("Malformed proof for " + table)
        missing = Counter(old) - Counter(current[table])
        n = sum(missing.values())
        additions = len(current[table]) - len(old) + n
        status = "FAIL" if n else "PASS"
        print(f"{status}: {table}: preserved={len(old)-n}/{len(old)} missing_or_changed={n} added={additions}")
        if n:
            passed = False
    return passed


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("capture", "verify"))
    parser.add_argument("--db", type=Path, required=True, help="Existing ci-watchtower.sqlite3, while app fully exited")
    parser.add_argument("--proof", type=Path, required=True, help="PRIVATE, uncommitted proof JSON file")
    opts = parser.parse_args(argv)
    try:
        db = opts.db.resolve()
        proof = opts.proof.resolve()
        if proof == db or proof in (Path(str(db) + "-wal"), Path(str(db) + "-shm")):
            raise ValueError("Proof path must not overwrite database or WAL/SHM files")
        if opts.mode == "capture":
            capture(db, proof)
            return 0
        return 0 if verify(db, proof) else 1
    except (ValueError, OSError, sqlite3.Error, json.JSONDecodeError) as exc:
        # No database contents or secrets are ever echoed.
        print(f"FAIL: preservation evidence unavailable: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
