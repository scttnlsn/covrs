use chrono::Utc;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{CovrsError, Result};
use crate::model::CoverageData;

pub const SCHEMA_VERSION: u32 = 4;

const SCHEMA: &str = include_str!("../schema.sql");

/// Open (or create) the covrs database at the given path.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
    Ok(conn)
}

/// Ensure the schema is initialized. Safe to call on an already-initialized DB.
/// Performs forward migrations when the on-disk schema version is older than
/// `SCHEMA_VERSION`.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;

    // Check or insert schema version
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    if count == 0 {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![SCHEMA_VERSION],
        )?;
    } else {
        let version: u32 = conn.query_row(
            "SELECT version FROM schema_version LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        if version > SCHEMA_VERSION {
            return Err(CovrsError::Other(format!(
                "Database schema version {} is newer than this binary supports ({}). \
                 Please upgrade covrs.",
                version, SCHEMA_VERSION
            )));
        }
        // Forward migration: apply each step from `version` to `SCHEMA_VERSION`.
        migrate(conn, version)?;
    }
    Ok(())
}

/// Apply migrations from `from_version` up to (and including) `SCHEMA_VERSION`.
/// Each migration step is a function that transforms the schema.
///
/// To add a new migration:
///   1. Bump `SCHEMA_VERSION`.
///   2. Add a new arm `N => { ... }` that migrates from version N to N+1.
///   3. Update schema.sql to reflect the final state (new installs skip migrations).
#[allow(unused_mut, unused_variables, clippy::never_loop)]
fn migrate(conn: &Connection, from_version: u32) -> Result<()> {
    let mut current = from_version;
    while current < SCHEMA_VERSION {
        eprintln!(
            "Migrating database schema from version {} to {} ...",
            current,
            current + 1
        );
        #[allow(clippy::match_single_binding)]
        match current {
            // Example migration steps (add real ones as schema evolves):
            // 3 => {
            //     conn.execute_batch("ALTER TABLE report ADD COLUMN metadata TEXT;")?;
            // }
            _ => {
                return Err(CovrsError::Other(format!(
                    "No migration path from schema version {} to {}. \
                     Consider deleting the database and re-ingesting.",
                    current,
                    current + 1
                )));
            }
        }
        // Note: when real migration arms are added above, they should not
        // return early — execution will fall through to here to bump the
        // version and continue.
        #[allow(unreachable_code)]
        {
            current += 1;
            conn.execute(
                "UPDATE schema_version SET version = ?1",
                params![current],
            )?;
        }
    }
    Ok(())
}

/// Insert a parsed `CoverageData` into the database under a new report.
/// Returns the report id.
pub fn insert_coverage(
    conn: &mut Connection,
    name: &str,
    source_format: &str,
    source_file: Option<&str>,
    data: &CoverageData,
) -> Result<i64> {
    let tx = conn.transaction()?;
    let report_id = insert_coverage_tx(&tx, name, source_format, source_file, data)?;
    tx.commit()?;
    Ok(report_id)
}

fn insert_coverage_tx(
    tx: &Transaction,
    name: &str,
    source_format: &str,
    source_file: Option<&str>,
    data: &CoverageData,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();

    tx.execute(
        "INSERT INTO report (name, source_format, source_file, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, source_format, source_file, now],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(ref err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            CovrsError::Other(format!(
                "Report '{}' already exists. Use --name to choose a different name, or delete it first.",
                name
            ))
        }
        other => CovrsError::Sqlite(other),
    })?;
    let report_id = tx.last_insert_rowid();

    // Cache source_file path -> id mappings
    let mut file_id_cache: HashMap<&str, i64> = HashMap::new();

    for file_cov in &data.files {
        let file_id = get_or_insert_source_file(tx, &file_cov.path, &mut file_id_cache)?;

        // Line coverage
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO line_coverage (report_id, source_file_id, line_number, hit_count) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for line in &file_cov.lines {
                stmt.execute(params![report_id, file_id, line.line_number, line.hit_count])?;
            }
        }

        // Branch coverage
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO branch_coverage (report_id, source_file_id, line_number, branch_index, hit_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for branch in &file_cov.branches {
                stmt.execute(params![
                    report_id,
                    file_id,
                    branch.line_number,
                    branch.branch_index,
                    branch.hit_count,
                ])?;
            }
        }

        // Function coverage — use upsert with COALESCE for NULL-safe dedup
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO function_coverage (report_id, source_file_id, name, start_line, end_line, hit_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(report_id, source_file_id, name, COALESCE(start_line, -1)) \
                 DO UPDATE SET hit_count = excluded.hit_count, end_line = excluded.end_line",
            )?;
            for func in &file_cov.functions {
                stmt.execute(params![
                    report_id,
                    file_id,
                    func.name,
                    func.start_line,
                    func.end_line,
                    func.hit_count,
                ])?;
            }
        }
    }

    Ok(report_id)
}

fn get_or_insert_source_file<'a>(
    tx: &Transaction,
    path: &'a str,
    cache: &mut HashMap<&'a str, i64>,
) -> Result<i64> {
    if let Some(&id) = cache.get(path) {
        return Ok(id);
    }
    tx.execute(
        "INSERT OR IGNORE INTO source_file (path) VALUES (?1)",
        params![path],
    )?;
    let id: i64 = tx.query_row(
        "SELECT id FROM source_file WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )?;
    cache.insert(path, id);
    Ok(id)
}

/// Merge report `source_name` into `target_name`, summing hit counts.
/// If `target_name` doesn't exist, creates it. Returns the target report id.
pub fn merge_reports(
    conn: &mut Connection,
    source_name: &str,
    target_name: &str,
) -> Result<i64> {
    let tx = conn.transaction()?;

    let source_id: i64 = tx
        .query_row(
            "SELECT id FROM report WHERE name = ?1",
            params![source_name],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::ReportNotFound(source_name.to_string()))?;

    // Find or create target report
    let target_id: i64 = match tx.query_row(
        "SELECT id FROM report WHERE name = ?1",
        params![target_name],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(id) => id,
        Err(_) => {
            let now = Utc::now().to_rfc3339();
            tx.execute(
                "INSERT INTO report (name, source_format, created_at) VALUES (?1, 'merged', ?2)",
                params![target_name, now],
            )?;
            tx.last_insert_rowid()
        }
    };

    // Merge line coverage: sum hit counts
    tx.execute(
        "INSERT INTO line_coverage (report_id, source_file_id, line_number, hit_count)
         SELECT ?1, source_file_id, line_number, hit_count
         FROM line_coverage WHERE report_id = ?2
         ON CONFLICT(report_id, source_file_id, line_number)
         DO UPDATE SET hit_count = hit_count + excluded.hit_count",
        params![target_id, source_id],
    )?;

    // Merge branch coverage: sum hit counts
    tx.execute(
        "INSERT INTO branch_coverage (report_id, source_file_id, line_number, branch_index, hit_count)
         SELECT ?1, source_file_id, line_number, branch_index, hit_count
         FROM branch_coverage WHERE report_id = ?2
         ON CONFLICT(report_id, source_file_id, line_number, branch_index)
         DO UPDATE SET hit_count = hit_count + excluded.hit_count",
        params![target_id, source_id],
    )?;

    // Merge function coverage: sum hit counts with NULL-safe start_line matching.
    // Step 1: Update existing matches in target.
    tx.execute(
        "UPDATE function_coverage AS target
         SET hit_count = target.hit_count + source.hit_count
         FROM (SELECT * FROM function_coverage WHERE report_id = ?2) AS source
         WHERE target.report_id = ?1
         AND target.source_file_id = source.source_file_id
         AND target.name = source.name
         AND target.start_line IS source.start_line",
        params![target_id, source_id],
    )?;
    // Step 2: Insert functions from source that have no match in target.
    tx.execute(
        "INSERT INTO function_coverage (report_id, source_file_id, name, start_line, end_line, hit_count)
         SELECT ?1, fc.source_file_id, fc.name, fc.start_line, fc.end_line, fc.hit_count
         FROM function_coverage fc
         WHERE fc.report_id = ?2
         AND NOT EXISTS (
             SELECT 1 FROM function_coverage t
             WHERE t.report_id = ?1
             AND t.source_file_id = fc.source_file_id
             AND t.name = fc.name
             AND t.start_line IS fc.start_line
         )",
        params![target_id, source_id],
    )?;

    tx.commit()?;
    Ok(target_id)
}

// ── Query helpers ──────────────────────────────────────────────────────────

/// Summary stats for a report.
#[derive(Debug)]
pub struct ReportSummary {
    pub report_name: String,
    pub source_format: String,
    pub created_at: String,
    pub total_files: u64,
    pub total_lines: u64,
    pub covered_lines: u64,
    pub total_branches: u64,
    pub covered_branches: u64,
    pub total_functions: u64,
    pub covered_functions: u64,
}

impl ReportSummary {
    pub fn line_rate(&self) -> f64 {
        if self.total_lines == 0 {
            return 0.0;
        }
        self.covered_lines as f64 / self.total_lines as f64
    }

    pub fn branch_rate(&self) -> f64 {
        if self.total_branches == 0 {
            return 0.0;
        }
        self.covered_branches as f64 / self.total_branches as f64
    }

    pub fn function_rate(&self) -> f64 {
        if self.total_functions == 0 {
            return 0.0;
        }
        self.covered_functions as f64 / self.total_functions as f64
    }
}

pub fn get_summary(conn: &Connection, report_name: &str) -> Result<ReportSummary> {
    conn.query_row(
        "SELECT
             r.name,
             r.source_format,
             r.created_at,
             (SELECT COUNT(DISTINCT source_file_id) FROM line_coverage WHERE report_id = r.id),
             (SELECT COUNT(*) FROM line_coverage WHERE report_id = r.id),
             (SELECT COUNT(*) FROM line_coverage WHERE report_id = r.id AND hit_count > 0),
             (SELECT COUNT(*) FROM branch_coverage WHERE report_id = r.id),
             (SELECT COUNT(*) FROM branch_coverage WHERE report_id = r.id AND hit_count > 0),
             (SELECT COUNT(*) FROM function_coverage WHERE report_id = r.id),
             (SELECT COUNT(*) FROM function_coverage WHERE report_id = r.id AND hit_count > 0)
         FROM report r
         WHERE r.name = ?1",
        params![report_name],
        |row| {
            Ok(ReportSummary {
                report_name: row.get(0)?,
                source_format: row.get(1)?,
                created_at: row.get(2)?,
                total_files: row.get(3)?,
                total_lines: row.get(4)?,
                covered_lines: row.get(5)?,
                total_branches: row.get(6)?,
                covered_branches: row.get(7)?,
                total_functions: row.get(8)?,
                covered_functions: row.get(9)?,
            })
        },
    )
    .map_err(|_| CovrsError::ReportNotFound(report_name.to_string()))
}

/// Per-file summary row.
#[derive(Debug)]
pub struct FileSummary {
    pub path: String,
    pub total_lines: u64,
    pub covered_lines: u64,
    pub total_branches: u64,
    pub covered_branches: u64,
}

impl FileSummary {
    pub fn line_rate(&self) -> f64 {
        if self.total_lines == 0 {
            return 0.0;
        }
        self.covered_lines as f64 / self.total_lines as f64
    }
}

pub fn get_file_summaries(conn: &Connection, report_name: &str) -> Result<Vec<FileSummary>> {
    let report_id: i64 = conn
        .query_row(
            "SELECT id FROM report WHERE name = ?1",
            params![report_name],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::ReportNotFound(report_name.to_string()))?;

    let mut stmt = conn.prepare(
        "SELECT sf.path,
                COUNT(*) as total,
                SUM(CASE WHEN lc.hit_count > 0 THEN 1 ELSE 0 END) as covered,
                COALESCE(bc.total_branches, 0),
                COALESCE(bc.covered_branches, 0)
         FROM line_coverage lc
         JOIN source_file sf ON sf.id = lc.source_file_id
         LEFT JOIN (
             SELECT source_file_id,
                    COUNT(*) as total_branches,
                    SUM(CASE WHEN hit_count > 0 THEN 1 ELSE 0 END) as covered_branches
             FROM branch_coverage
             WHERE report_id = ?1
             GROUP BY source_file_id
         ) bc ON bc.source_file_id = lc.source_file_id
         WHERE lc.report_id = ?1
         GROUP BY sf.path
         ORDER BY sf.path",
    )?;

    let rows = stmt.query_map(params![report_id], |row| {
        Ok(FileSummary {
            path: row.get(0)?,
            total_lines: row.get(1)?,
            covered_lines: row.get(2)?,
            total_branches: row.get(3)?,
            covered_branches: row.get(4)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Line-level detail for a source file in a report.
#[derive(Debug)]
pub struct LineDetail {
    pub line_number: u32,
    pub hit_count: u64,
}

pub fn get_lines(
    conn: &Connection,
    report_name: &str,
    source_path: &str,
) -> Result<Vec<LineDetail>> {
    let report_id: i64 = conn
        .query_row(
            "SELECT id FROM report WHERE name = ?1",
            params![report_name],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::ReportNotFound(report_name.to_string()))?;

    let source_file_id: i64 = conn
        .query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![source_path],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::Other(format!("Source file not found: {}", source_path)))?;

    let mut stmt = conn.prepare(
        "SELECT line_number, hit_count FROM line_coverage
         WHERE report_id = ?1 AND source_file_id = ?2
         ORDER BY line_number",
    )?;

    let rows = stmt.query_map(params![report_id, source_file_id], |row| {
        Ok(LineDetail {
            line_number: row.get(0)?,
            hit_count: row.get(1)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// List all report names in the database.
pub fn list_reports(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT name, source_format, created_at FROM report ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Check whether a report with the given name exists.
pub fn report_exists(conn: &Connection, name: &str) -> Result<bool> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM report WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Return the name of the most recently created report, if any.
pub fn get_latest_report_name(conn: &Connection) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM report ORDER BY created_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// Delete a report and all its associated coverage data.
/// Coverage rows are removed automatically via ON DELETE CASCADE.
pub fn delete_report(conn: &mut Connection, report_name: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let report_id: i64 = tx
        .query_row(
            "SELECT id FROM report WHERE name = ?1",
            params![report_name],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::ReportNotFound(report_name.to_string()))?;

    tx.execute("DELETE FROM report WHERE id = ?1", params![report_id])?;

    // Clean up orphaned source_file rows no longer referenced by any coverage data
    tx.execute(
        "DELETE FROM source_file WHERE id NOT IN (
             SELECT DISTINCT source_file_id FROM line_coverage
             UNION
             SELECT DISTINCT source_file_id FROM branch_coverage
             UNION
             SELECT DISTINCT source_file_id FROM function_coverage
         )",
        [],
    )?;

    tx.commit()?;
    Ok(())
}

/// Compute coverage for lines touched by a diff.
/// `diff_lines` is a map of source_path -> set of line numbers from the diff.
/// Returns (covered, total) for only those lines.
pub fn diff_coverage(
    conn: &Connection,
    report_name: &str,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(u64, u64)> {
    let report_id: i64 = conn
        .query_row(
            "SELECT id FROM report WHERE name = ?1",
            params![report_name],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::ReportNotFound(report_name.to_string()))?;

    let mut total: u64 = 0;
    let mut covered: u64 = 0;

    for (path, lines) in diff_lines {
        let file_id: i64 = match conn.query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![path],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => continue, // file not in coverage data
        };

        if lines.is_empty() {
            continue;
        }

        // Batch queries to stay within SQLite's parameter limit.
        // Reserve 2 slots for report_id and file_id.
        const BATCH_SIZE: usize = 500;

        for chunk in lines.chunks(BATCH_SIZE) {
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT line_number, hit_count FROM line_coverage \
                 WHERE report_id = ?1 AND source_file_id = ?2 AND line_number IN ({})",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;

            // Build parameter list: report_id, file_id, then each line number
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(report_id));
            param_values.push(Box::new(file_id));
            for &ln in chunk {
                param_values.push(Box::new(ln));
            }
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let rows = stmt.query_map(params_ref.as_slice(), |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?))
            })?;

            for row in rows {
                let (_line_number, hit_count) = row?;
                total += 1;
                if hit_count > 0 {
                    covered += 1;
                }
            }
        }
    }

    Ok((covered, total))
}
