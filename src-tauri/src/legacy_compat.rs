use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

const LEGACY_PROJECT_SCOPE_MIGRATION: &str = "legacy-project-scope-v032";
const CORE_DECOUPLING_MIGRATION: &str = "core-decoupling-v032";

fn migration_applied(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE migration_key=?)",
        params![key],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?)
}

fn mark_migration(conn: &Connection, key: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations(migration_key,applied_at) VALUES(?,?)",
        params![key, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(super) fn needs_project_scope_migration(conn: &Connection) -> Result<bool> {
    if migration_applied(conn, LEGACY_PROJECT_SCOPE_MIGRATION)? {
        return Ok(false);
    }

    let unscoped_repositories: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM monitored_repositories WHERE project_id IS NULL)",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    let unscoped_tracks: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM watch_tracks WHERE project_id IS NULL)",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    let legacy_tracks_without_projects: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tracks) AND NOT EXISTS(SELECT 1 FROM projects)",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;

    Ok(unscoped_repositories || unscoped_tracks || legacy_tracks_without_projects)
}

pub(super) fn migrate_project_scope(conn: &Connection) -> Result<()> {
    migrate_legacy(conn)?;
    migrate_project_scope_legacy(conn)
}

pub(super) fn finish_project_scope_migration(conn: &Connection) -> Result<()> {
    seed_myeongha_aliases(conn)?;
    seed_bejewely_project_scope(conn)?;
    mark_migration(conn, LEGACY_PROJECT_SCOPE_MIGRATION)
}

pub(super) fn mark_core_decoupling(conn: &Connection) -> Result<()> {
    mark_migration(conn, CORE_DECOUPLING_MIGRATION)
}

pub(super) fn legacy_track_key(name: &str, id: i64) -> String {
    let lower = name.to_lowercase();
    if lower.contains("프론트") || lower.contains("frontend") {
        "frontend-integration".into()
    } else if lower.contains("관상 연구") || lower.contains("face research") {
        "face-research".into()
    } else if lower.contains("관상") || lower.contains("face reading") {
        "face-reading".into()
    } else if lower.contains("사주") || lower.contains("saju") {
        "saju".into()
    } else if lower.contains("운영") || lower.contains("ops") {
        "ops".into()
    } else if lower.contains("결제") || lower.contains("commerce") {
        "product-commerce".into()
    } else if lower.contains("파이프라인") || lower.contains("reliability") {
        "pipeline-reliability".into()
    } else {
        format!("legacy-{id}")
    }
}

fn migrate_legacy(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id,name,repo,long_ci_minutes,created_at,updated_at FROM tracks ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, name, repo, long_ci, created_at, updated_at) = row?;
        let key = legacy_track_key(&name, id);
        conn.execute(
            "INSERT OR IGNORE INTO watch_tracks(name,track_key,long_ci_minutes,active,legacy_track_id,created_at,updated_at) VALUES(?,?,?,1,?,?,?)",
            params![name, key, long_ci, id, created_at, updated_at],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO monitored_repositories(repo,enabled,created_at,updated_at) VALUES(?,1,?,?)",
            params![repo, now, now],
        )?;
    }
    Ok(())
}

pub(super) fn invalidate_cross_project_assignments(
    conn: &Connection,
    repository_id: i64,
    from_repository_project_id: Option<i64>,
    to_repository_project_id: i64,
    migration_key: &str,
) -> Result<()> {
    let migrated_at = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO assignment_migration_audit(
           migration_key,run_id,repository_id,repository_repo,
           from_repository_project_id,to_repository_project_id,
           track_id,track_project_id,track_key,track_name,
           confidence,source,reason,manual,assigned_at,action,migrated_at
         )
         SELECT ?,wr.run_id,wr.repository_id,mr.repo,?,?,
                ra.track_id,wt.project_id,wt.track_key,wt.name,
                ra.confidence,ra.source,ra.reason,ra.manual,ra.assigned_at,
                CASE
                  WHEN ra.manual=1 THEN 'manual-invalidated-for-project-move'
                  ELSE 'automatic-invalidated-for-project-move'
                END,
                ?
         FROM run_assignments ra
         JOIN workflow_runs wr ON wr.run_id=ra.run_id
         JOIN monitored_repositories mr ON mr.id=wr.repository_id
         JOIN watch_tracks wt ON wt.id=ra.track_id
         WHERE wr.repository_id=?
           AND wt.project_id<>?",
        params![
            migration_key,
            from_repository_project_id,
            to_repository_project_id,
            migrated_at,
            repository_id,
            to_repository_project_id
        ],
    )?;

    conn.execute(
        "DELETE FROM run_assignments
         WHERE run_id IN (
           SELECT wr.run_id
           FROM workflow_runs wr
           WHERE wr.repository_id=?
         )
         AND track_id IN (
           SELECT id FROM watch_tracks WHERE project_id<>?
         )",
        params![repository_id, to_repository_project_id],
    )?;

    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='assigned'
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id
           )",
        params![repository_id],
    )?;

    Ok(())
}

pub(super) fn seed_bejewely_project_scope(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();

    tx.execute(
        "INSERT OR IGNORE INTO projects(name,project_key,active,created_at,updated_at)
         VALUES('비주얼리','visualy',1,?,?)",
        params![now, now],
    )?;
    let project_id: i64 = tx.query_row(
        "SELECT id FROM projects WHERE project_key='visualy' LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE projects SET name='비주얼리',active=1,updated_at=? WHERE id=?",
        params![now, project_id],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO monitored_repositories(
           project_id,repo,enabled,running_count,queued_count,created_at,updated_at
         ) VALUES(?,'gycha0109-beep/K_beauty',1,0,0,?,?)",
        params![project_id, now, now],
    )?;
    let (repository_id, previous_repository_project_id): (i64, Option<i64>) = tx.query_row(
        "SELECT id,project_id
         FROM monitored_repositories
         WHERE repo='gycha0109-beep/K_beauty'
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    invalidate_cross_project_assignments(
        &tx,
        repository_id,
        previous_repository_project_id,
        project_id,
        "visualy-project-scope-v1",
    )?;

    tx.execute(
        "UPDATE monitored_repositories SET project_id=?,updated_at=?
         WHERE id=?",
        params![project_id, now, repository_id],
    )?;

    for (name, track_key, long_ci_minutes) in [
        ("CI Watchtower / CI 운영 정리", "ops", 8_i64),
        ("데이터 정렬 & AI", "taxonomy-ai", 8_i64),
        ("신규 상품 신뢰도 운영 파이프라인", "trust", 8_i64),
        ("Face Lab 연구", "face-research", 8_i64),
        ("Premium Full Report", "full-report", 8_i64),
        ("Mobile", "mobile", 8_i64),
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO watch_tracks(
               project_id,name,track_key,long_ci_minutes,active,created_at,updated_at
             ) VALUES(?,?,?,?,1,?,?)",
            params![project_id, name, track_key, long_ci_minutes, now, now],
        )?;
    }

    for workflow_name in ["BEJEWELY Current Main Health", "PIE Prospective Shadow"] {
        tx.execute(
            "INSERT OR IGNORE INTO project_workflow_rules(
               project_id,repository_id,workflow_name,active,created_at
             ) VALUES(?,NULL,?,1,?)",
            params![project_id, workflow_name, now],
        )?;
    }

    // Visualy's repository-owned responsibility map is authoritative for producer class.
    // Security/Supply Chain were previously seeded as Project-wide, but are now
    // explicitly shared-dynamic producers. Remove the stale classification and
    // return affected non-manual runs to fail-closed reconciliation.
    tx.execute(
        "DELETE FROM project_workflow_rules
         WHERE project_id=?
           AND workflow_name IN ('BEJEWELY Security Boundary','BEJEWELY Supply Chain Security')",
        params![project_id],
    )?;
    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='project'
           AND workflow_name IN ('BEJEWELY Security Boundary','BEJEWELY Supply Chain Security')
           AND NOT EXISTS(
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        params![repository_id],
    )?;

    for workflow_name in [
        "Admin - Access Foundation",
        "Admin - Product Current Main Integration",
        "Product Offer Runtime - DATA-OFFER17 Controlled RPC Diagnostic",
        "Product Data Pipeline - Hwahae Provenance Non-Main PR Guard",
        "Product Data Pipeline - Identity Key Repair Confirm",
        "Product Data Pipeline - Product Offers",
        "Product Data Pipeline - Source Bindings",
        "BEJEWELY Security Boundary",
        "Recommendation Admission - G3A PF Authority Read",
        "BEJEWELY Supply Chain Security",
        "BEJEWELY AI Provider Runtime",
        "BEJEWELY Database Integration Authority",
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO dynamic_workflow_rules(
               project_id,repository_id,workflow_name,active,protected,created_at
             ) VALUES(?,?,?,1,1,?)",
            params![project_id, repository_id, workflow_name, now],
        )?;
        tx.execute(
            "UPDATE dynamic_workflow_rules
             SET active=1,protected=1
             WHERE project_id=? AND repository_id=? AND workflow_name=?",
            params![project_id, repository_id, workflow_name],
        )?;
    }

    tx.execute(
        "DELETE FROM run_assignments
         WHERE manual=0
           AND run_id IN (
             SELECT wr.run_id
             FROM workflow_runs wr
             JOIN project_workflow_rules pwr
               ON pwr.project_id=?
              AND pwr.active=1
              AND pwr.workflow_name=wr.workflow_name
              AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
             WHERE wr.repository_id=?
           )",
        params![project_id, repository_id],
    )?;

    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='project'
         WHERE repository_id=?
           AND ignored=0
           AND EXISTS (
             SELECT 1
             FROM project_workflow_rules pwr
             WHERE pwr.project_id=?
               AND pwr.active=1
               AND pwr.workflow_name=workflow_runs.workflow_name
               AND (pwr.repository_id IS NULL OR pwr.repository_id=workflow_runs.repository_id)
           )
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra
             WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        params![repository_id, project_id],
    )?;

    tx.execute(
        "UPDATE workflow_runs
         SET resolution_status='unassigned'
         WHERE repository_id=?
           AND ignored=0
           AND resolution_status='assigned'
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id
           )",
        params![repository_id],
    )?;

    tx.commit()?;
    Ok(())
}

pub(super) fn migrate_project_scope_legacy(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();

    let has_myeongha: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM monitored_repositories WHERE repo LIKE '%/MyeongHa' OR repo LIKE '%/Saju')",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;

    let project_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
    if project_count == 0 {
        let (name, key) = if has_myeongha {
            ("명하", "myeongha")
        } else {
            ("기본 프로젝트", "default")
        };
        conn.execute(
            "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES(?,?,1,?,?)",
            params![name, key, now, now],
        )?;
    }

    let default_project_id: i64 = if has_myeongha {
        conn.query_row(
            "SELECT id FROM projects WHERE project_key='myeongha' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .or_else(|_| {
            conn.execute(
                "INSERT INTO projects(name,project_key,active,created_at,updated_at) VALUES('명하','myeongha',1,?,?)",
                params![now, now],
            )?;
            Ok::<i64, rusqlite::Error>(conn.last_insert_rowid())
        })?
    } else {
        conn.query_row(
            "SELECT id FROM projects WHERE active=1 ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?
    };

    conn.execute(
        "UPDATE monitored_repositories SET project_id=? WHERE project_id IS NULL",
        params![default_project_id],
    )?;
    conn.execute(
        "UPDATE watch_tracks SET project_id=? WHERE project_id IS NULL",
        params![default_project_id],
    )?;

    let commerce_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM watch_tracks WHERE track_key='commerce' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let product_commerce_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM watch_tracks WHERE track_key='product-commerce')",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    if let Some(id) = commerce_id {
        if !product_commerce_exists {
            conn.execute(
                "UPDATE watch_tracks SET track_key='product-commerce',updated_at=? WHERE id=?",
                params![now, id],
            )?;
        }
    }

    if has_myeongha {
        let myeongha_project_id: i64 = conn.query_row(
            "SELECT project_id FROM monitored_repositories WHERE repo LIKE '%/MyeongHa' OR repo LIKE '%/Saju' ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        for workflow_name in [
            "CI",
            "Governance",
            "Web PR Domain Gates",
            "PIE Prospective Shadow",
        ] {
            conn.execute(
                "INSERT OR IGNORE INTO project_workflow_rules(project_id,repository_id,workflow_name,active,created_at)
                 VALUES(?,NULL,?,1,?)",
                params![myeongha_project_id, workflow_name, now],
            )?;
        }

        let myeongha_repository_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/MyeongHa'
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(repository_id) = myeongha_repository_id {
            for workflow_name in [
                "DB Content Reading Suite",
                "DB Runtime Authority Suite",
                "DB PostgreSQL 17 Authority Suite",
                "Supabase Production",
                "Web Browser Smoke",
                "Web Auth Browser Regression",
            ] {
                conn.execute(
                    "INSERT OR IGNORE INTO project_workflow_rules(
                       project_id,repository_id,workflow_name,active,created_at
                     ) VALUES(?,?,?,1,?)",
                    params![myeongha_project_id, repository_id, workflow_name, now],
                )?;
            }

            conn.execute(
                "INSERT OR IGNORE INTO dynamic_workflow_rules(
                   project_id,repository_id,workflow_name,active,protected,created_at
                 ) VALUES(?,?,'Production Records Current-Subject Smoke',1,1,?)",
                params![myeongha_project_id, repository_id, now],
            )?;
            conn.execute(
                "UPDATE dynamic_workflow_rules
                 SET active=1,protected=1
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='Production Records Current-Subject Smoke'",
                params![myeongha_project_id, repository_id],
            )?;
        }

        let saju_repository_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM monitored_repositories
                 WHERE repo='gycha0109-beep/Saju'
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(repository_id) = saju_repository_id {
            conn.execute(
                "INSERT OR IGNORE INTO dynamic_workflow_rules(
                   project_id,repository_id,workflow_name,active,protected,created_at
                 ) VALUES(?,?,'MESH6J Manual Browser Capture Surface CI',1,1,?)",
                params![myeongha_project_id, repository_id, now],
            )?;
            conn.execute(
                "UPDATE dynamic_workflow_rules
                 SET active=1,protected=1
                 WHERE project_id=? AND repository_id=?
                   AND workflow_name='MESH6J Manual Browser Capture Surface CI'",
                params![myeongha_project_id, repository_id],
            )?;
        }
    }

    conn.execute(
        "DELETE FROM run_assignments
         WHERE manual=0 AND run_id IN (
           SELECT wr.run_id
           FROM workflow_runs wr
           JOIN monitored_repositories mr ON mr.id=wr.repository_id
           JOIN project_workflow_rules pwr
             ON pwr.project_id=mr.project_id
            AND pwr.active=1
            AND pwr.workflow_name=wr.workflow_name
            AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
         )",
        [],
    )?;
    conn.execute(
        "UPDATE workflow_runs
         SET resolution_status='project'
         WHERE ignored=0
           AND run_id IN (
             SELECT wr.run_id
             FROM workflow_runs wr
             JOIN monitored_repositories mr ON mr.id=wr.repository_id
             JOIN project_workflow_rules pwr
               ON pwr.project_id=mr.project_id
              AND pwr.active=1
              AND pwr.workflow_name=wr.workflow_name
              AND (pwr.repository_id IS NULL OR pwr.repository_id=wr.repository_id)
           )
           AND NOT EXISTS (
             SELECT 1 FROM run_assignments ra WHERE ra.run_id=workflow_runs.run_id AND ra.manual=1
           )",
        [],
    )?;

    Ok(())
}

fn seed_myeongha_aliases(conn: &Connection) -> Result<()> {
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM projects WHERE project_key='myeongha' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let now = Utc::now().to_rfc3339();

    for (alias_key, track_key) in [
        ("privacy-recovery", "ops"),
        ("commerce", "product-commerce"),
    ] {
        let track_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM watch_tracks
                 WHERE project_id=? AND track_key=? AND active=1
                 LIMIT 1",
                params![project_id, track_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(track_id) = track_id {
            conn.execute(
                "INSERT INTO track_aliases(project_id,alias_key,track_id,active,created_at)
                 VALUES(?,?,?,1,?)
                 ON CONFLICT(project_id,alias_key)
                 DO UPDATE SET track_id=excluded.track_id,active=1",
                params![project_id, alias_key, track_id, now],
            )?;
        }
    }
    Ok(())
}
