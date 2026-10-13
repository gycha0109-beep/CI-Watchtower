#!/usr/bin/env python3
"""Deterministic fixtures for the read-only P1 preservation probe. No real app DB."""
import contextlib
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

import verify_watchtower_db as probe


class PreservationTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.db = base / "synthetic.sqlite3"
        self.proof = base / "private-baseline.json"
        connection = sqlite3.connect(self.db)
        for table, fields in probe.PROTECTED.items():
            columns = ", ".join(f'"{column}" TEXT' for column in fields)
            connection.execute(f'CREATE TABLE "{table}" ({columns})')
            values = [f"synthetic-{table}-{field}" for field in fields]
            if table == "run_assignments":
                values[fields.index("manual")] = "1"
            placeholders = ",".join("?" for _ in fields)
            connection.execute(f'INSERT INTO "{table}" VALUES ({placeholders})', values)
        connection.commit()
        connection.close()

    def call(self, *args):
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream), contextlib.redirect_stderr(stream):
            result = probe.main(list(args))
        return result, stream.getvalue()

    def baseline(self):
        code, out = self.call("capture", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 0, out)
        self.assertTrue(self.proof.exists())
        payload = json.loads(self.proof.read_text("utf-8"))
        self.assertEqual(set(payload["tables"]), set(probe.PROTECTED))
        self.assertNotIn("synthetic-projects-", self.proof.read_text("utf-8"))
        return payload

    def test_unchanged_passes_and_baseline_cannot_be_overwritten(self):
        self.baseline()
        code, out = self.call("verify", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 0, out)
        code, _ = self.call("capture", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 1)

    def test_new_poll_runs_and_mutable_status_do_not_falsely_fail(self):
        self.baseline()
        connection = sqlite3.connect(self.db)
        # New Run/Attempt rows are allowed; existing IDs/Head SHA remain immutable.
        run_fields = probe.PROTECTED["workflow_runs"]
        placeholders = ",".join("?" for _ in run_fields)
        connection.execute(f"INSERT INTO workflow_runs VALUES ({placeholders})",
                           ["new-run-" + field for field in run_fields])
        connection.execute("ALTER TABLE run_attempts ADD COLUMN status TEXT")
        connection.execute("UPDATE run_attempts SET status='completed'")
        connection.commit()
        connection.close()
        code, out = self.call("verify", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 0, out)
        self.assertIn("added=1", out)

    def test_deletion_and_mutation_fail_closed(self):
        self.baseline()
        connection = sqlite3.connect(self.db)
        connection.execute("DELETE FROM notifications_v2")
        connection.execute("UPDATE watch_tracks SET track_key='lost-association'")
        connection.commit()
        connection.close()
        code, out = self.call("verify", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 1)
        self.assertIn("FAIL: notifications_v2:", out)
        self.assertIn("FAIL: watch_tracks:", out)
        self.assertNotIn("synthetic-", out)

    def test_manual_assignment_retention(self):
        self.baseline()
        connection = sqlite3.connect(self.db)
        connection.execute("UPDATE run_assignments SET track_id='wrong-track'")
        connection.commit()
        connection.close()
        code, out = self.call("verify", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 1, out)
        self.assertIn("FAIL: run_assignments:", out)

    def test_missing_database_does_not_create_file(self):
        missing = self.db.with_name("never-created.sqlite3")
        code, out = self.call("capture", "--db", str(missing), "--proof", str(self.proof))
        self.assertEqual(code, 1)
        self.assertFalse(missing.exists())
        self.assertFalse(self.proof.exists())

    def test_missing_table_fails_instead_of_silent_success(self):
        connection = sqlite3.connect(self.db)
        connection.execute("DROP TABLE run_attempts")
        connection.commit()
        connection.close()
        code, out = self.call("capture", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 1)
        self.assertIn("run_attempts", out)
        self.assertFalse(self.proof.exists())

    def test_no_proof_can_replace_a_database_or_overwrite_baseline(self):
        code, _ = self.call("capture", "--db", str(self.db), "--proof", str(self.db))
        self.assertEqual(code, 1)
        self.assertTrue(self.db.exists())
        self.baseline()
        proof = json.loads(self.proof.read_text("utf-8"))
        proof["tables"].pop("notifications_v2")
        self.proof.write_text(json.dumps(proof), encoding="utf-8")
        code, _ = self.call("verify", "--db", str(self.db), "--proof", str(self.proof))
        self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
