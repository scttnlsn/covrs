//! Command handler functions for the covrs CLI.
//!
//! Each `cmd_*` function returns its output as a `String`, making them easy
//! to test without capturing stdout.

use std::fmt::Write;
use std::path::Path;

use anyhow::Result;
use clap::ValueEnum;
use rusqlite::Connection;

use crate::{db, diff, report};

/// Output style for the `diff-coverage` command.
#[derive(Clone, ValueEnum)]
pub enum Style {
    Text,
    Markdown,
}

pub fn cmd_ingest(
    conn: &mut Connection,
    file: &Path,
    format: Option<&str>,
    name: Option<&str>,
    overwrite: bool,
) -> Result<String> {
    let (report_id, detected_format, actual_name) =
        crate::ingest::ingest(conn, file, format, name, overwrite)?;
    Ok(format!(
        "Ingested {} as format '{}' → report id {} (name: '{}')\n",
        file.display(),
        detected_format,
        report_id,
        actual_name,
    ))
}

pub fn cmd_summary(conn: &Connection) -> Result<String> {
    let summary = db::get_summary(conn)?;

    let mut out = String::new();
    writeln!(out, "Files:      {}", summary.total_files).unwrap();
    writeln!(
        out,
        "Lines:      {}/{} ({:.1}%)",
        summary.covered_lines,
        summary.total_lines,
        summary.line_rate() * 100.0
    )
    .unwrap();
    if summary.total_branches > 0 {
        writeln!(
            out,
            "Branches:   {}/{} ({:.1}%)",
            summary.covered_branches,
            summary.total_branches,
            summary.branch_rate() * 100.0
        )
        .unwrap();
    }
    if summary.total_functions > 0 {
        writeln!(
            out,
            "Functions:  {}/{} ({:.1}%)",
            summary.covered_functions,
            summary.total_functions,
            summary.function_rate() * 100.0
        )
        .unwrap();
    }
    Ok(out)
}

pub fn cmd_reports(conn: &Connection) -> Result<String> {
    let reports = db::list_reports(conn)?;
    if reports.is_empty() {
        return Ok("No reports in database.\n".to_string());
    }
    let mut out = String::new();
    writeln!(out, "{:<30} {:<15} CREATED", "NAME", "FORMAT").unwrap();
    writeln!(out, "{}", "-".repeat(70)).unwrap();
    for (name, format, created) in &reports {
        writeln!(out, "{:<30} {:<15} {}", name, format, created).unwrap();
    }
    Ok(out)
}

pub fn cmd_files(conn: &Connection, sort_by_coverage: bool) -> Result<String> {
    let mut files = db::get_file_summaries(conn)?;

    if sort_by_coverage {
        files.sort_by(|a, b| a.line_rate().total_cmp(&b.line_rate()));
    }

    let mut out = String::new();
    writeln!(
        out,
        "{:<60} {:>8} {:>8} {:>8}",
        "FILE", "LINES", "COVERED", "RATE"
    )
    .unwrap();
    writeln!(out, "{}", "-".repeat(88)).unwrap();

    for f in &files {
        writeln!(
            out,
            "{:<60} {:>8} {:>8} {:>7.1}%",
            f.path,
            f.total_lines,
            f.covered_lines,
            f.line_rate() * 100.0
        )
        .unwrap();
    }

    Ok(out)
}

pub fn cmd_lines(conn: &Connection, source_file: &str, uncovered: bool) -> Result<String> {
    let lines = db::get_lines(conn, source_file)?;

    if uncovered {
        let uncovered_lines: Vec<_> = lines.iter().filter(|l| l.hit_count == 0).collect();

        if uncovered_lines.is_empty() {
            return Ok(format!(
                "All instrumentable lines are covered in '{}'\n",
                source_file
            ));
        }

        let mut out = String::new();
        writeln!(out, "Uncovered lines in '{}':", source_file).unwrap();
        let uncovered_numbers: Vec<u32> = uncovered_lines.iter().map(|l| l.line_number).collect();
        writeln!(out, "  {}", report::format_line_ranges(&uncovered_numbers)).unwrap();
        writeln!(out, "  ({} lines)", uncovered_lines.len()).unwrap();
        Ok(out)
    } else {
        if lines.is_empty() {
            return Ok(format!("No coverage data for '{}'\n", source_file));
        }

        let mut out = String::new();
        writeln!(out, "{:>6}  {:>10}", "LINE", "HITS").unwrap();
        writeln!(out, "{}", "-".repeat(18)).unwrap();
        for line in &lines {
            let marker = if line.hit_count > 0 { "✓" } else { "✗" };
            writeln!(
                out,
                "{:>6}  {:>10}  {}",
                line.line_number, line.hit_count, marker
            )
            .unwrap();
        }
        Ok(out)
    }
}

/// Core diff-coverage logic. Accepts the diff text directly so callers can
/// obtain it from stdin, `git diff`, or the GitHub API.
pub fn cmd_diff_coverage(
    conn: &Connection,
    diff_text: &str,
    path_prefix: Option<&str>,
    style: &Style,
    sha: Option<String>,
) -> Result<String> {
    let mut diff_lines = diff::parse_diff(diff_text);

    if let Some(prefix) = path_prefix {
        diff_lines = diff::apply_path_prefix(diff_lines, prefix);
    }

    let report = report::build_report(conn, &diff_lines, sha)?;

    let output = match style {
        Style::Text => report.format_text(),
        Style::Markdown => report.format_markdown(),
    };

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CoverageData, FileCoverage, FunctionCoverage, LineCoverage};

    /// Create an in-memory database with schema initialized.
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        db::init_schema(&conn).unwrap();
        conn
    }

    /// Insert sample coverage data: two files, some lines covered, some not,
    /// plus a function entry.
    fn seed_coverage(conn: &mut Connection) {
        let data = CoverageData {
            files: vec![
                FileCoverage {
                    path: "src/main.rs".to_string(),
                    lines: vec![
                        LineCoverage {
                            line_number: 1,
                            hit_count: 5,
                        },
                        LineCoverage {
                            line_number: 2,
                            hit_count: 3,
                        },
                        LineCoverage {
                            line_number: 3,
                            hit_count: 0,
                        },
                        LineCoverage {
                            line_number: 4,
                            hit_count: 0,
                        },
                    ],
                    branches: vec![],
                    functions: vec![FunctionCoverage {
                        name: "main".to_string(),
                        start_line: Some(1),
                        end_line: Some(4),
                        hit_count: 5,
                    }],
                },
                FileCoverage {
                    path: "src/lib.rs".to_string(),
                    lines: vec![
                        LineCoverage {
                            line_number: 1,
                            hit_count: 10,
                        },
                        LineCoverage {
                            line_number: 2,
                            hit_count: 10,
                        },
                    ],
                    branches: vec![],
                    functions: vec![],
                },
            ],
        };
        db::insert_coverage(conn, "test-report", "lcov", None, &data, false).unwrap();
    }

    #[test]
    fn test_cmd_summary() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_summary(&conn).unwrap();

        assert!(out.contains("Files:      2"));
        assert!(out.contains("Lines:      4/6"));
        assert!(out.contains("66.7%"));
        assert!(out.contains("Functions:  1/1"));
    }

    #[test]
    fn test_cmd_reports() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_reports(&conn).unwrap();

        assert!(out.contains("NAME"));
        assert!(out.contains("test-report"));
        assert!(out.contains("lcov"));
    }

    #[test]
    fn test_cmd_reports_empty() {
        let conn = test_db();

        let out = cmd_reports(&conn).unwrap();

        assert!(out.contains("No reports in database."));
    }

    #[test]
    fn test_cmd_files() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_files(&conn, false).unwrap();

        assert!(out.contains("src/main.rs"));
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("100.0%"));
        assert!(out.contains("50.0%"));
    }

    #[test]
    fn test_cmd_files_sorted_by_coverage() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_files(&conn, true).unwrap();

        // When sorted ascending by coverage, src/main.rs (50%) should appear
        // before src/lib.rs (100%).
        let main_pos = out.find("src/main.rs").unwrap();
        let lib_pos = out.find("src/lib.rs").unwrap();
        assert!(main_pos < lib_pos);
    }

    #[test]
    fn test_cmd_lines() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_lines(&conn, "src/main.rs", false).unwrap();

        assert!(out.contains("LINE"));
        assert!(out.contains("HITS"));
        assert!(out.contains("✓"));
        assert!(out.contains("✗"));
    }

    #[test]
    fn test_cmd_lines_no_data() {
        let conn = test_db();

        let result = cmd_lines(&conn, "nonexistent.rs", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_cmd_lines_uncovered() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_lines(&conn, "src/main.rs", true).unwrap();

        assert!(out.contains("Uncovered lines in 'src/main.rs':"));
        assert!(out.contains("3-4"));
        assert!(out.contains("2 lines"));
    }

    #[test]
    fn test_cmd_lines_uncovered_all_covered() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_lines(&conn, "src/lib.rs", true).unwrap();

        assert!(out.contains("All instrumentable lines are covered"));
    }

    #[test]
    fn test_cmd_diff_coverage_text() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let diff_text = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -0,0 +1,4 @@
+fn main() {
+    let x = 1;
+    let y = 2;
+    let z = 3;
";

        let out = cmd_diff_coverage(&conn, diff_text, None, &Style::Text, None).unwrap();

        assert!(out.contains("Patch coverage:"));
        assert!(out.contains("50.0%"));
    }

    #[test]
    fn test_cmd_diff_coverage_markdown() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let diff_text = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -0,0 +1,4 @@
+fn main() {
+    let x = 1;
+    let y = 2;
+    let z = 3;
";

        let out = cmd_diff_coverage(
            &conn,
            diff_text,
            None,
            &Style::Markdown,
            Some("abc1234".to_string()),
        )
        .unwrap();

        assert!(out.contains("## Patch Coverage:"));
        assert!(out.contains("abc1234"));
    }

    #[test]
    fn test_cmd_diff_coverage_empty_diff() {
        let mut conn = test_db();
        seed_coverage(&mut conn);

        let out = cmd_diff_coverage(&conn, "", None, &Style::Text, None).unwrap();

        assert!(out.contains("No added lines found in diff."));
    }

    #[test]
    fn test_cmd_diff_coverage_with_path_prefix() {
        let mut conn = test_db();

        let data = CoverageData {
            files: vec![FileCoverage {
                path: "project/app.rs".to_string(),
                lines: vec![
                    LineCoverage {
                        line_number: 1,
                        hit_count: 1,
                    },
                    LineCoverage {
                        line_number: 2,
                        hit_count: 0,
                    },
                ],
                branches: vec![],
                functions: vec![],
            }],
        };
        db::insert_coverage(&mut conn, "prefix-report", "lcov", None, &data, false).unwrap();

        let diff_text = "\
diff --git a/app.rs b/app.rs
--- a/app.rs
+++ b/app.rs
@@ -0,0 +1,2 @@
+line one
+line two
";

        let out = cmd_diff_coverage(&conn, diff_text, Some("project"), &Style::Text, None).unwrap();

        assert!(out.contains("Patch coverage:"));
        assert!(out.contains("1/2"));
    }

    #[test]
    fn test_cmd_ingest() {
        let mut conn = test_db();

        let dir = tempfile::tempdir().unwrap();
        let lcov_path = dir.path().join("test.lcov");
        std::fs::write(&lcov_path, "SF:src/foo.rs\nDA:1,5\nDA:2,0\nend_of_record\n").unwrap();

        let out = cmd_ingest(&mut conn, &lcov_path, None, Some("my-report"), false).unwrap();

        assert!(out.contains("Ingested"));
        assert!(out.contains("lcov"));
        assert!(out.contains("my-report"));

        // Verify data actually made it into the DB
        let reports = db::list_reports(&conn).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].0, "my-report");
    }
}
