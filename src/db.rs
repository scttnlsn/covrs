use chrono::Utc;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Result};

use crate::model::{
    CoverageData, FileCoverage, FileDiffCoverage, FileSummary, LineDetail, ReportInfo,
    ReportSummary,
};

const SCHEMA: &str = include_str!("../schema.sql");

/// Open (or create) the covrs database at the given path.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;
    conn.execute_batch("PRAGMA cache_size=-65536;")?; // 64 MB page cache
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
    insert_coverage_streaming(conn, name, source_format, source_file, overwrite, |emit| {
        for file in &data.files {
            emit(file)?;
        }
        Ok(())
    })
}

/// Streaming variant of [`insert_coverage`]. Instead of taking a finished
/// `CoverageData`, accepts a closure that receives an `emit` callback.
/// The closure should call `emit` once per source file. This lets callers
/// pipe parsed files directly into the database without collecting them
/// all into memory first.
pub fn insert_coverage_streaming(
    conn: &mut Connection,
    name: &str,
    source_format: &str,
    source_file: Option<&str>,
    overwrite: bool,
    with_files: impl FnOnce(&mut dyn FnMut(&FileCoverage) -> Result<()>) -> Result<()>,
) -> Result<i64> {
    let tx = conn.transaction()?;

    if overwrite {
        tx.execute("DELETE FROM report WHERE name = ?1", params![name])?;
    }

    let report_id = insert_coverage_tx(&tx, name, source_format, source_file, with_files)?;

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

/// Maximum rows per multi-row INSERT batch. Kept well under SQLite's
/// default `SQLITE_MAX_VARIABLE_NUMBER` (32 766 for bundled builds).
/// 2 000 rows × 6 params (the widest statement) = 12 000 parameters.
const INSERT_BATCH_SIZE: usize = 2000;

/// Accumulates rows and flushes them as batched multi-row INSERT statements
/// to reduce per-row overhead.
struct BatchInsert<'a> {
    tx: &'a Transaction<'a>,
    /// SQL prefix, e.g. `INSERT OR REPLACE INTO t (a, b) VALUES`.
    prefix: &'static str,
    /// Optional SQL suffix appended after the VALUES clause (e.g. ON CONFLICT).
    suffix: &'static str,
    /// Number of columns per row.
    cols: usize,
    /// Flat list of parameter values for the current batch.
    params: Vec<rusqlite::types::Value>,
    /// Number of complete rows in the current batch.
    rows: usize,
    /// Whether `flush` has been called after the last `push_row`.
    flushed: bool,
}

impl<'a> BatchInsert<'a> {
    fn new(
        tx: &'a Transaction<'a>,
        prefix: &'static str,
        suffix: &'static str,
        cols: usize,
    ) -> Self {
        Self {
            tx,
            prefix,
            suffix,
            cols,
            params: Vec::with_capacity(INSERT_BATCH_SIZE * cols),
            rows: 0,
            flushed: true,
        }
    }

    /// Append one complete row. Flushes automatically when the batch is full.
    /// Takes ownership of values to avoid an extra clone.
    fn push_row<I: IntoIterator<Item = rusqlite::types::Value>>(
        &mut self,
        values: I,
    ) -> Result<()> {
        let iter = values.into_iter();
        let (min, _) = iter.size_hint();
        debug_assert_eq!(min, self.cols, "wrong number of columns");
        self.params.extend(iter);
        self.rows += 1;
        self.flushed = false;
        if self.rows >= INSERT_BATCH_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    /// Flush any remaining rows. Must be called after the last `push_row`.
    fn flush(&mut self) -> Result<()> {
        if self.rows == 0 {
            self.flushed = true;
            return Ok(());
        }
        debug_assert_eq!(self.params.len(), self.rows * self.cols);
        let values_clause = multi_row_values(self.rows, self.cols);
        let sql = format!("{} {values_clause}{}", self.prefix, self.suffix);
        self.tx
            .execute(&sql, rusqlite::params_from_iter(self.params.iter()))?;
        self.params.clear();
        self.rows = 0;
        self.flushed = true;
        Ok(())
    }
}

impl Drop for BatchInsert<'_> {
    fn drop(&mut self) {
        debug_assert!(self.flushed, "BatchInsert dropped with unflushed rows");
    }
}

fn opt_u32(v: Option<u32>) -> rusqlite::types::Value {
    match v {
        Some(n) => rusqlite::types::Value::Integer(n as i64),
        None => rusqlite::types::Value::Null,
    }
}

fn insert_coverage_tx(
    tx: &Transaction,
    name: &str,
    source_format: &str,
    source_file: Option<&str>,
    with_files: impl FnOnce(&mut dyn FnMut(&FileCoverage) -> Result<()>) -> Result<()>,
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

    let mut file_id_cache: HashMap<String, i64> = HashMap::new();

    let mut lines = BatchInsert::new(
        tx,
        "INSERT OR REPLACE INTO line_coverage (report_id, source_file_id, line_number, hit_count) VALUES",
        "",
        4,
    );
    let mut branches = BatchInsert::new(
        tx,
        "INSERT OR REPLACE INTO branch_coverage (report_id, source_file_id, line_number, branch_index, hit_count) VALUES",
        "",
        5,
    );
    let mut functions = BatchInsert::new(
        tx,
        "INSERT INTO function_coverage (report_id, source_file_id, name, start_line, end_line, hit_count) VALUES",
        " ON CONFLICT(report_id, source_file_id, name, COALESCE(start_line, -1)) \
         DO UPDATE SET hit_count = excluded.hit_count, end_line = excluded.end_line",
        6,
    );

    with_files(&mut |file_cov: &FileCoverage| {
        let file_id = get_or_insert_source_file_owned(tx, &file_cov.path, &mut file_id_cache)?;
        let rid = rusqlite::types::Value::Integer(report_id);
        let fid = rusqlite::types::Value::Integer(file_id);

        for line in &file_cov.lines {
            lines.push_row([
                rid.clone(),
                fid.clone(),
                (line.line_number as i64).into(),
                (line.hit_count as i64).into(),
            ])?;
        }
        for branch in &file_cov.branches {
            branches.push_row([
                rid.clone(),
                fid.clone(),
                (branch.line_number as i64).into(),
                (branch.branch_index as i64).into(),
                (branch.hit_count as i64).into(),
            ])?;
        }
        for func in &file_cov.functions {
            functions.push_row([
                rid.clone(),
                fid.clone(),
                func.name.clone().into(),
                opt_u32(func.start_line),
                opt_u32(func.end_line),
                (func.hit_count as i64).into(),
            ])?;
        }
        Ok(())
    })?;

    lines.flush()?;
    branches.flush()?;
    functions.flush()?;

    Ok(report_id)
}

/// Generate a VALUES clause like `(?,?,?),(?,?,?),...` for `rows` rows of
/// `cols` columns each, using positional (`?`) placeholders.
fn multi_row_values(rows: usize, cols: usize) -> String {
    // Build the single-row template "(?,?,...,?)" without allocating a Vec.
    let mut single = String::with_capacity(2 + cols * 2);
    single.push('(');
    for i in 0..cols {
        if i > 0 {
            single.push(',');
        }
        single.push('?');
    }
    single.push(')');

    let mut out = String::with_capacity((single.len() + 1) * rows);
    for i in 0..rows {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&single);
    }
    out
}

fn get_or_insert_source_file_owned(
    tx: &Transaction,
    path: &str,
    cache: &mut HashMap<String, i64>,
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
    cache.insert(path.to_owned(), id);
    Ok(id)
}

// ── Query helpers ──────────────────────────────────────────────────────────

/// Returns true when there are multiple reports in the database, meaning
/// queries must use GROUP BY / MAX(hit_count) to implement union semantics
/// (a line is covered if ANY report covers it). When there is at most one
/// report every (source_file_id, line_number) tuple is already unique
/// (enforced by the primary key) so the grouping can be skipped.
fn needs_union(conn: &Connection) -> Result<bool> {
    let count: u32 = conn.query_row("SELECT COUNT(*) FROM report", [], |row| row.get(0))?;
    Ok(count > 1)
}

/// Which coverage table to build a union source for.
enum UnionKind {
    Line,
    /// Line source that also projects source_file_id for per-file grouping.
    LinePerFile,
    Branch,
    /// Branch source that also projects source_file_id for per-file grouping.
    BranchPerFile,
    Function,
}

/// Returns a SQL fragment (table name or subquery) that collapses duplicate
/// rows via MAX(hit_count) when `union` is true, or the raw table when false.
fn union_source(union: bool, kind: UnionKind) -> &'static str {
    match (union, kind) {
        (false, UnionKind::Line | UnionKind::LinePerFile) => "line_coverage",
        (true, UnionKind::Line) => {
            "(SELECT source_file_id, MAX(hit_count) AS hit_count \
              FROM line_coverage GROUP BY source_file_id, line_number)"
        }
        (true, UnionKind::LinePerFile) => {
            "(SELECT source_file_id, line_number, MAX(hit_count) AS hit_count \
              FROM line_coverage GROUP BY source_file_id, line_number)"
        }
        (false, UnionKind::Branch | UnionKind::BranchPerFile) => "branch_coverage",
        (true, UnionKind::Branch) => {
            "(SELECT MAX(hit_count) AS hit_count \
              FROM branch_coverage GROUP BY source_file_id, line_number, branch_index)"
        }
        (true, UnionKind::BranchPerFile) => {
            "(SELECT source_file_id, MAX(hit_count) AS hit_count \
              FROM branch_coverage GROUP BY source_file_id, line_number, branch_index)"
        }
        (false, UnionKind::Function) => "function_coverage",
        (true, UnionKind::Function) => {
            "(SELECT MAX(hit_count) AS hit_count \
              FROM function_coverage GROUP BY source_file_id, name, COALESCE(start_line, -1))"
        }
    }
}

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
pub fn diff_coverage(
    conn: &Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
) -> Result<(Vec<FileDiffCoverage>, usize, usize)> {
    let union = needs_union(conn)?;
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

            let sql = if union {
                format!(
                    r#"SELECT line_number, MAX(hit_count) FROM line_coverage
                     WHERE source_file_id = ? AND line_number IN ({placeholders})
                     GROUP BY line_number"#
                )
            } else {
                format!(
                    r#"SELECT line_number, hit_count FROM line_coverage
                     WHERE source_file_id = ? AND line_number IN ({placeholders})"#
                )
            };
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
    let report_count: u32 = conn.query_row("SELECT COUNT(*) FROM report", [], |row| row.get(0))?;
    if report_count == 0 {
        bail!("No reports in database. Run 'covrs ingest' first.");
    }

    // When there is only one report every (source_file_id, line_number)
    // tuple is already unique (enforced by the PK) so we can skip the
    // GROUP BY / MAX(hit_count) subqueries.
    let union = report_count > 1;
    let line_src = union_source(union, UnionKind::Line);
    let branch_src = union_source(union, UnionKind::Branch);
    let function_src = union_source(union, UnionKind::Function);

    let (total_files, total_lines, covered_lines): (u64, u64, u64) = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT source_file_id), COUNT(*),
                    COALESCE(SUM(CASE WHEN hit_count > 0 THEN 1 ELSE 0 END), 0)
             FROM {line_src}"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    let (total_branches, covered_branches): (u64, u64) = conn.query_row(
        &format!(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN hit_count > 0 THEN 1 ELSE 0 END), 0)
             FROM {branch_src}"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let (total_functions, covered_functions): (u64, u64) = conn.query_row(
        &format!(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN hit_count > 0 THEN 1 ELSE 0 END), 0)
             FROM {function_src}"
        ),
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
    let union = needs_union(conn)?;
    let line_src = union_source(union, UnionKind::LinePerFile);
    let branch_src = union_source(union, UnionKind::BranchPerFile);

    let sql = format!(
        "SELECT sf.path,
                COUNT(*) AS total,
                SUM(CASE WHEN lc.hit_count > 0 THEN 1 ELSE 0 END) AS covered,
                COALESCE(bc.total_branches, 0),
                COALESCE(bc.covered_branches, 0)
         FROM {line_src} lc
         JOIN source_file sf ON sf.id = lc.source_file_id
         LEFT JOIN (
             SELECT source_file_id,
                    COUNT(*) AS total_branches,
                    SUM(CASE WHEN hit_count > 0 THEN 1 ELSE 0 END) AS covered_branches
             FROM {branch_src}
             GROUP BY source_file_id
         ) bc ON bc.source_file_id = lc.source_file_id
         GROUP BY sf.path
         ORDER BY sf.path"
    );

    let mut stmt = conn.prepare(&sql)?;

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

/// Total line coverage rate for a single file (union semantics).
///
/// Returns `None` if the file is not in the database.
pub fn get_file_line_rate(conn: &Connection, path: &str) -> Result<Option<f64>> {
    let union = needs_union(conn)?;
    let line_src = union_source(union, UnionKind::LinePerFile);

    let sql = format!(
        "SELECT COUNT(*) AS total,
                SUM(CASE WHEN lc.hit_count > 0 THEN 1 ELSE 0 END) AS covered
         FROM {line_src} lc
         JOIN source_file sf ON sf.id = lc.source_file_id
         WHERE sf.path = ?1"
    );

    let (total, covered): (u64, u64) =
        conn.query_row(&sql, params![path], |row| Ok((row.get(0)?, row.get(1)?)))?;

    if total == 0 {
        Ok(None)
    } else {
        Ok(Some(crate::model::rate(covered, total)))
    }
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

    let mut stmt = if needs_union(conn)? {
        conn.prepare(
            "SELECT line_number, MAX(hit_count) AS hit_count
             FROM line_coverage
             WHERE source_file_id = ?1
             GROUP BY line_number
             ORDER BY line_number",
        )?
    } else {
        conn.prepare(
            "SELECT line_number, hit_count
             FROM line_coverage
             WHERE source_file_id = ?1
             ORDER BY line_number",
        )?
    };

    let rows = stmt.query_map(params![source_file_id], |row| {
        Ok(LineDetail {
            line_number: row.get(0)?,
            hit_count: row.get(1)?,
        })
    })?;

    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
