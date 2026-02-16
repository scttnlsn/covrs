use chrono::Utc;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{CovrsError, Result};
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

/// Summary stats across all reports in the database.
#[derive(Debug)]
pub struct ReportSummary {
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

/// Line-level detail for a source file.
#[derive(Debug)]
pub struct LineDetail {
    pub line_number: u32,
    pub hit_count: u64,
}

/// List all report names in the database.
pub fn list_reports(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt =
        conn.prepare("SELECT name, source_format, created_at FROM report ORDER BY created_at")?;
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

/// Per-file patch coverage detail.
#[derive(Debug)]
pub struct FileDiffCoverage {
    pub path: String,
    /// Diff lines that are instrumentable and covered.
    pub covered_lines: Vec<u32>,
    /// Diff lines that are instrumentable and NOT covered.
    pub missed_lines: Vec<u32>,
}

impl FileDiffCoverage {
    pub fn total(&self) -> usize {
        self.covered_lines.len() + self.missed_lines.len()
    }
}

/// Compute per-file patch coverage detail for lines touched by a diff,
/// considering ALL reports in the database. A line is covered if any report
/// has a hit_count > 0 for it.
///
/// Returns a vec of per-file results (only files that have at least one
/// instrumentable diff line), plus (total_covered, total_instrumentable).
pub fn diff_coverage_detail(
    conn: &Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(Vec<FileDiffCoverage>, u64, u64)> {
    let mut results: Vec<FileDiffCoverage> = Vec::new();
    let mut total_covered: u64 = 0;
    let mut total_instrumentable: u64 = 0;

    for (path, lines) in diff_lines {
        let file_id: i64 = match conn.query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![path],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => continue,
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
                 WHERE source_file_id = ?1 AND line_number IN ({}) \
                 GROUP BY line_number",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
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

        total_covered += covered.len() as u64;
        total_instrumentable += (covered.len() + missed.len()) as u64;

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
        return Err(CovrsError::Other(
            "No reports in database. Run 'covrs ingest' first.".to_string(),
        ));
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

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Line-level detail for a source file across all reports (union semantics).
pub fn get_lines(conn: &Connection, source_path: &str) -> Result<Vec<LineDetail>> {
    let source_file_id: i64 = conn
        .query_row(
            "SELECT id FROM source_file WHERE path = ?1",
            params![source_path],
            |row| row.get(0),
        )
        .map_err(|_| CovrsError::Other(format!("Source file not found: {}", source_path)))?;

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

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Compute coverage for diff lines across all reports (union semantics).
/// Returns (covered, total) for only instrumentable diff lines.
pub fn diff_coverage(
    conn: &Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(u64, u64)> {
    let (_, covered, total) = diff_coverage_detail(conn, diff_lines)?;
    Ok((covered, total))
}

/// Compute the overall line coverage rate across all reports in the database.
/// A line is covered if any report has a hit_count > 0 for it.
pub fn get_overall_line_rate(conn: &Connection) -> Result<Option<f64>> {
    let (total, covered): (u64, u64) = conn.query_row(
        "SELECT COUNT(*), SUM(CASE WHEN max_hits > 0 THEN 1 ELSE 0 END)
         FROM (
             SELECT MAX(hit_count) as max_hits
             FROM line_coverage
             GROUP BY source_file_id, line_number
         )",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if total == 0 {
        return Ok(None);
    }
    Ok(Some(covered as f64 / total as f64))
}
