use chrono::Utc;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Result};

use crate::model::{FileDiffCoverage, FileSummary, LineDetail, ReportInfo, ReportSummary};

use crate::model::CoverageData;

const SCHEMA: &str = include_str!("../schema.sql");

/// Open (or create) the covrs database at the given path.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

/// Ensure the schema is initialized. Safe to call on an already-initialized DB.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Insert a parsed `CoverageData` into the database under a new report.
/// If `overwrite` is true, any existing report with the same name is replaced.
/// Returns the report id.
pub fn insert_coverage(
    conn: &mut Connection,
    name: &str,
    source_format: &str,
    source_file: Option<&str>,
    data: &CoverageData,
    overwrite: bool,
) -> Result<i64> {
    let tx = conn.transaction()?;

    if overwrite {
        tx.execute("DELETE FROM report WHERE name = ?1", params![name])?;
    }

    let report_id = insert_coverage_tx(&tx, name, source_format, source_file, data)?;

    if overwrite {
        // Clean up source files orphaned by the delete
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
    }

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
            anyhow::anyhow!(
                "Report '{name}' already exists. Use --name to choose a different name, or delete it first."
            )
        }
        other => anyhow::Error::from(other),
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
                stmt.execute(params![
                    report_id,
                    file_id,
                    line.line_number,
                    line.hit_count
                ])?;
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
    let id: i64 = tx.query_row(
        "INSERT INTO source_file (path) VALUES (?1) \
         ON CONFLICT(path) DO UPDATE SET path = path \
         RETURNING id",
        params![path],
        |row| row.get(0),
    )?;
    cache.insert(path, id);
    Ok(id)
}

// ── Query helpers ──────────────────────────────────────────────────────────

/// List all reports in the database.
pub fn list_reports(conn: &Connection) -> Result<Vec<ReportInfo>> {
    let mut stmt =
        conn.prepare("SELECT name, source_format, created_at FROM report ORDER BY created_at")?;
    let rows = stmt.query_map([], |row| {
        Ok(ReportInfo {
            name: row.get(0)?,
            format: row.get(1)?,
            created_at: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Compute per-file diff coverage detail for lines touched by a diff,
/// considering ALL reports in the database. A line is covered if any report
/// has a hit_count > 0 for it.
///
/// Returns a vec of per-file results (only files that have at least one
/// instrumentable diff line), plus (total_covered, total_instrumentable).
pub fn diff_coverage_detail(
    conn: &Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(Vec<FileDiffCoverage>, usize, usize)> {
    let mut results: Vec<FileDiffCoverage> = Vec::new();
    let mut total_covered: usize = 0;
    let mut total_instrumentable: usize = 0;

    for (path, lines) in diff_lines {
        let file_id: i64 = match conn.query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![path],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.into()),
        };

        if lines.is_empty() {
            continue;
        }

        let mut covered: Vec<u32> = Vec::new();
        let mut missed: Vec<u32> = Vec::new();

        const BATCH_SIZE: usize = 500;
        for chunk in lines.chunks(BATCH_SIZE) {
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT line_number, MAX(hit_count) FROM line_coverage \
                 WHERE source_file_id = ? AND line_number IN ({placeholders}) \
                 GROUP BY line_number"
            );
            let mut stmt = conn.prepare(&sql)?;

            let params: Vec<rusqlite::types::Value> =
                std::iter::once(rusqlite::types::Value::Integer(file_id))
                    .chain(
                        chunk
                            .iter()
                            .map(|&ln| rusqlite::types::Value::Integer(i64::from(ln))),
                    )
                    .collect();

            let rows = stmt.query_map(rusqlite::params_from_iter(&params), |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?))
            })?;

            for row in rows {
                let (line_number, hit_count) = row?;
                if hit_count > 0 {
                    covered.push(line_number);
                } else {
                    missed.push(line_number);
                }
            }
        }

        if covered.is_empty() && missed.is_empty() {
            continue;
        }

        covered.sort();
        missed.sort();

        total_covered += covered.len();
        total_instrumentable += covered.len() + missed.len();

        results.push(FileDiffCoverage {
            path: path.clone(),
            covered_lines: covered,
            missed_lines: missed,
        });
    }

    results.sort_by(|a, b| a.path.cmp(&b.path));

    Ok((results, total_covered, total_instrumentable))
}

/// Summary across all reports (union semantics: a line/branch/function is
/// covered if ANY report covers it).
pub fn get_summary(conn: &Connection) -> Result<ReportSummary> {
    let count: u32 = conn.query_row("SELECT COUNT(*) FROM report", [], |row| row.get(0))?;
    if count == 0 {
        bail!("No reports in database. Run 'covrs ingest' first.");
    }

    let (total_files, total_lines, covered_lines): (u64, u64, u64) = conn.query_row(
        "SELECT
             COUNT(DISTINCT source_file_id),
             COUNT(*),
             COALESCE(SUM(CASE WHEN max_hits > 0 THEN 1 ELSE 0 END), 0)
         FROM (
             SELECT source_file_id, line_number, MAX(hit_count) as max_hits
             FROM line_coverage
             GROUP BY source_file_id, line_number
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    let (total_branches, covered_branches): (u64, u64) = conn.query_row(
        "SELECT
             COUNT(*),
             COALESCE(SUM(CASE WHEN max_hits > 0 THEN 1 ELSE 0 END), 0)
         FROM (
             SELECT MAX(hit_count) as max_hits
             FROM branch_coverage
             GROUP BY source_file_id, line_number, branch_index
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let (total_functions, covered_functions): (u64, u64) = conn.query_row(
        "SELECT
             COUNT(*),
             COALESCE(SUM(CASE WHEN max_hits > 0 THEN 1 ELSE 0 END), 0)
         FROM (
             SELECT MAX(hit_count) as max_hits
             FROM function_coverage
             GROUP BY source_file_id, name, COALESCE(start_line, -1)
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    Ok(ReportSummary {
        total_files,
        total_lines,
        covered_lines,
        total_branches,
        covered_branches,
        total_functions,
        covered_functions,
    })
}

/// Per-file coverage summaries across all reports (union semantics).
pub fn get_file_summaries(conn: &Connection) -> Result<Vec<FileSummary>> {
    let mut stmt = conn.prepare(
        "SELECT sf.path,
                COUNT(*) as total,
                SUM(CASE WHEN lc.max_hits > 0 THEN 1 ELSE 0 END) as covered,
                COALESCE(bc.total_branches, 0),
                COALESCE(bc.covered_branches, 0)
         FROM (
             SELECT source_file_id, line_number, MAX(hit_count) as max_hits
             FROM line_coverage
             GROUP BY source_file_id, line_number
         ) lc
         JOIN source_file sf ON sf.id = lc.source_file_id
         LEFT JOIN (
             SELECT source_file_id,
                    COUNT(*) as total_branches,
                    SUM(CASE WHEN max_hits > 0 THEN 1 ELSE 0 END) as covered_branches
             FROM (
                 SELECT source_file_id, line_number, branch_index, MAX(hit_count) as max_hits
                 FROM branch_coverage
                 GROUP BY source_file_id, line_number, branch_index
             )
             GROUP BY source_file_id
         ) bc ON bc.source_file_id = lc.source_file_id
         GROUP BY sf.path
         ORDER BY sf.path",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(FileSummary {
            path: row.get(0)?,
            total_lines: row.get(1)?,
            covered_lines: row.get(2)?,
            total_branches: row.get(3)?,
            covered_branches: row.get(4)?,
        })
    })?;

    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Line-level detail for a source file across all reports (union semantics).
pub fn get_lines(conn: &Connection, source_path: &str) -> Result<Vec<LineDetail>> {
    let source_file_id: i64 = conn
        .query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![source_path],
            |row| row.get(0),
        )
        .map_err(|_| anyhow::anyhow!("Source file not found: {source_path}"))?;

    let mut stmt = conn.prepare(
        "SELECT line_number, MAX(hit_count) as hit_count
         FROM line_coverage
         WHERE source_file_id = ?1
         GROUP BY line_number
         ORDER BY line_number",
    )?;

    let rows = stmt.query_map(params![source_file_id], |row| {
        Ok(LineDetail {
            line_number: row.get(0)?,
            hit_count: row.get(1)?,
        })
    })?;

    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Compute coverage for diff lines across all reports (union semantics).
/// Returns (covered, total) for only instrumentable diff lines.
pub fn diff_coverage(
    conn: &Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(usize, usize)> {
    let (_, covered, total) = diff_coverage_detail(conn, diff_lines)?;
    Ok((covered, total))
}
