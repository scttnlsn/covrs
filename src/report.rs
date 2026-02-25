//! Output formatting for diff coverage results.

use std::collections::HashMap;
use std::fmt::Write;

use crate::model::{rate, FileDiffCoverage};

/// Aggregated diff coverage data, ready to be formatted.
pub struct DiffCoverageReport {
    /// Number of added lines per file from the diff.
    pub diff_files: usize,
    /// Total number of added lines across all files.
    pub diff_lines: usize,
    /// Per-file coverage detail (only files with at least one instrumentable line).
    pub files: Vec<FileDiffCoverage>,
    /// Total instrumentable diff lines that are covered.
    pub total_covered: usize,
    /// Total instrumentable diff lines.
    pub total_instrumentable: usize,
    /// Overall project line coverage rate (if available).
    pub total_rate: Option<f64>,
    /// Per-file total line coverage rates (path → rate as 0.0–1.0).
    pub file_rates: HashMap<String, f64>,
    /// Commit SHA to display.
    pub sha: Option<String>,
}

impl DiffCoverageReport {
    /// Format using a specific formatter.
    #[must_use]
    pub fn format(&self, formatter: &dyn ReportFormatter) -> String {
        formatter.format(self)
    }
}

/// Trait for formatting diff coverage reports.
pub trait ReportFormatter {
    /// Format the report to a string.
    fn format(&self, report: &DiffCoverageReport) -> String;
}

/// Plain text formatter.
pub struct TextFormatter;

impl ReportFormatter for TextFormatter {
    fn format(&self, report: &DiffCoverageReport) -> String {
        let mut out = String::new();

        if report.diff_files == 0 {
            out.push_str("No added lines found in diff.\n");
            return out;
        }

        if report.total_instrumentable == 0 {
            let lines = report.diff_lines;
            let files = report.diff_files;
            writeln!(
                out,
                "{lines} lines added across {files} files — none are instrumentable."
            )
            .unwrap();
            return out;
        }

        let pct = rate(
            report.total_covered as u64,
            report.total_instrumentable as u64,
        ) * 100.0;
        let covered = report.total_covered;
        let total = report.total_instrumentable;
        writeln!(
            out,
            "Diff coverage: {pct:.1}% ({covered}/{total} lines covered)"
        )
        .unwrap();

        let mut files_with_misses: Vec<_> = report
            .files
            .iter()
            .filter(|f| !f.missed_lines.is_empty())
            .collect();
        files_with_misses.sort_by(|a, b| a.rate().partial_cmp(&b.rate()).unwrap());
        if !files_with_misses.is_empty() {
            out.push('\n');
            for f in &files_with_misses {
                let file_total = f.total();
                let file_covered = f.covered_lines.len();
                let file_rate = f.rate() * 100.0;
                let path = &f.path;
                let all_instrumentable = f.all_instrumentable();
                let missed = format_line_ranges(&f.missed_lines, &all_instrumentable);
                writeln!(
                    out,
                    "  {path}  {file_covered}/{file_total} ({file_rate:.1}%)  missed: {missed}",
                )
                .unwrap();
            }
        }

        if let Some(rate) = report.total_rate {
            out.push('\n');
            let pct = rate * 100.0;
            writeln!(out, "Full project coverage: {pct:.1}%").unwrap();
        }

        out
    }
}

/// Markdown formatter.
pub struct MarkdownFormatter;

impl ReportFormatter for MarkdownFormatter {
    fn format(&self, report: &DiffCoverageReport) -> String {
        let mut md = String::new();

        let diff_rate = rate(
            report.total_covered as u64,
            report.total_instrumentable as u64,
        ) * 100.0;

        writeln!(md, "### Diff Coverage: {diff_rate:.1}%\n").unwrap();

        let covered = report.total_covered;
        let total = report.total_instrumentable;
        write!(md, "**{covered}** of **{total}** diff lines covered").unwrap();
        if let Some(ref sha) = report.sha {
            let short_sha = if sha.len() > 7 { &sha[..7] } else { sha };
            write!(md, " ({short_sha})").unwrap();
        }
        md.push('\n');

        let mut files_with_misses: Vec<&FileDiffCoverage> = report
            .files
            .iter()
            .filter(|f| !f.missed_lines.is_empty())
            .collect();
        files_with_misses.sort_by(|a, b| a.rate().partial_cmp(&b.rate()).unwrap());

        if files_with_misses.is_empty() {
            md.push_str("\nAll diff lines are covered! 🎉\n");
        } else {
            md.push_str("\n| File | Missed | Diff | Total | \n");
            md.push_str("|:-----|-------:|-----:|------:|\n");

            for f in &files_with_misses {
                let file_rate = f.rate() * 100.0;
                let path = &f.path;
                let missed_count = f.missed_lines.len();
                let total_rate = report.file_rates.get(path).copied().unwrap_or(0.0) * 100.0;
                writeln!(
                    md,
                    "| `{path}` | {missed_count} | {file_rate:.0}% | {total_rate:.0}% |"
                )
                .unwrap();
            }

            md.push_str("\n<details>\n<summary>Missed lines</summary>\n\n");

            for f in &files_with_misses {
                let path = &f.path;
                let all_instrumentable = f.all_instrumentable();
                let ranges = if let Some(ref sha) = report.sha {
                    format_line_ranges_linked(&f.missed_lines, &all_instrumentable, sha, path)
                } else {
                    format_line_ranges(&f.missed_lines, &all_instrumentable)
                };
                writeln!(md, "**`{path}`**: {ranges}\n").unwrap();
            }

            md.push_str("</details>\n");
        }

        md.push('\n');
        if let Some(rate) = report.total_rate {
            let pct = rate * 100.0;
            writeln!(md, "<sub>Full project coverage: **{pct:.1}%**</sub>").unwrap();
        }
        md.push_str("<sub>[covrs](https://github.com/scttnlsn/covrs)</sub>\n");

        md
    }
}

/// Build a [`DiffCoverageReport`] from parsed diff lines and a database connection.
pub fn build_report(
    conn: &rusqlite::Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
    sha: Option<&str>,
) -> anyhow::Result<DiffCoverageReport> {
    let diff_files = diff_lines.len();
    let diff_line_count: usize = diff_lines.values().map(|v| v.len()).sum();

    let (files, total_covered, total_instrumentable) = if diff_lines.is_empty() {
        (vec![], 0, 0)
    } else {
        crate::db::diff_coverage(conn, diff_lines)?
    };

    let total_rate = match crate::db::get_summary(conn) {
        Ok(s) if s.total_lines > 0 => Some(s.line_rate()),
        Ok(_) => None,
        Err(e) => {
            eprintln!("Warning: could not compute project coverage: {e}");
            None
        }
    };

    let mut file_rates = HashMap::new();
    for f in &files {
        match crate::db::get_file_line_rate(conn, &f.path) {
            Ok(Some(r)) => {
                file_rates.insert(f.path.clone(), r);
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("Warning: could not compute coverage for {}: {e}", f.path);
            }
        }
    }

    Ok(DiffCoverageReport {
        diff_files,
        diff_lines: diff_line_count,
        files,
        total_covered,
        total_instrumentable,
        total_rate,
        file_rates,
        sha: sha.map(|s| s.to_owned()),
    })
}

/// Maximum number of consecutive non-instrumentable lines that can be bridged
/// when coalescing uncovered ranges. Gaps of up to this many lines (where none
/// of the gap lines are instrumentable) are merged into a single range.
const MAX_BRIDGE_GAP: u32 = 2;

/// Coalesce sorted line numbers into `(start, end)` ranges, bridging small
/// gaps where every line in the gap is non-instrumentable.
///
/// A gap between two uncovered lines is bridged only when:
/// 1. Every line in the gap is absent from `all_instrumentable`, AND
/// 2. The gap is at most [`MAX_BRIDGE_GAP`] lines wide.
///
/// Both `lines` and `all_instrumentable` must be sorted and deduplicated.
#[must_use]
pub fn coalesce_ranges(lines: &[u32], all_instrumentable: &[u32]) -> Vec<(u32, u32)> {
    if lines.is_empty() {
        return Vec::new();
    }

    debug_assert!(
        lines.windows(2).all(|w| w[0] < w[1]),
        "coalesce_ranges requires sorted, deduplicated input"
    );

    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut start = lines[0];
    let mut end = lines[0];

    for &line in &lines[1..] {
        let gap = line - end - 1;
        if gap <= MAX_BRIDGE_GAP
            && (end + 1..line).all(|l| all_instrumentable.binary_search(&l).is_err())
        {
            end = line;
        } else {
            ranges.push((start, end));
            start = line;
            end = line;
        }
    }

    ranges.push((start, end));
    ranges
}

/// Format line numbers into compact range notation with markdown links.
///
/// Each line number becomes a link like `[N](../blob/{sha}/{path}#LN)`.
/// Ranges are rendered as `[3-5](../blob/{sha}/{path}#L3-L5)`.
///
/// The input slice must be sorted in ascending order.
#[must_use]
pub fn format_line_ranges_linked(
    lines: &[u32],
    all_instrumentable: &[u32],
    sha: &str,
    path: &str,
) -> String {
    let ranges = coalesce_ranges(lines, all_instrumentable);

    let link = |start: u32, end: u32| -> String {
        if start == end {
            format!("[{start}](../blob/{sha}/{path}#L{start})")
        } else {
            format!("[{start}-{end}](../blob/{sha}/{path}#L{start}-L{end})")
        }
    };

    ranges
        .iter()
        .map(|&(start, end)| link(start, end))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format line numbers into compact range notation, e.g. "1, 3-5, 8".
///
/// The input slice must be sorted in ascending order.
#[must_use]
pub fn format_line_ranges(lines: &[u32], all_instrumentable: &[u32]) -> String {
    let ranges = coalesce_ranges(lines, all_instrumentable);

    ranges
        .iter()
        .map(|&(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- coalesce_ranges tests -----------------------------------------------

    #[test]
    fn test_coalesce_ranges_empty() {
        assert_eq!(coalesce_ranges(&[], &[]), Vec::<(u32, u32)>::new());
    }

    #[test]
    fn test_coalesce_ranges_single() {
        assert_eq!(coalesce_ranges(&[5], &[5]), vec![(5, 5)]);
    }

    #[test]
    fn test_coalesce_ranges_consecutive() {
        assert_eq!(coalesce_ranges(&[1, 2, 3], &[1, 2, 3]), vec![(1, 3)]);
    }

    #[test]
    fn test_coalesce_ranges_bridges_one_non_instrumentable() {
        // Lines 1,2,4,5 uncovered; line 3 not instrumentable → bridge
        assert_eq!(coalesce_ranges(&[1, 2, 4, 5], &[1, 2, 4, 5]), vec![(1, 5)]);
    }

    #[test]
    fn test_coalesce_ranges_bridges_two_non_instrumentable() {
        // Lines 1,2,5,6 uncovered; lines 3,4 not instrumentable → bridge
        assert_eq!(coalesce_ranges(&[1, 2, 5, 6], &[1, 2, 5, 6]), vec![(1, 6)]);
    }

    #[test]
    fn test_coalesce_ranges_no_bridge_three_non_instrumentable() {
        // Lines 1,2,6,7 uncovered; lines 3,4,5 not instrumentable → too wide, no bridge
        assert_eq!(
            coalesce_ranges(&[1, 2, 6, 7], &[1, 2, 6, 7]),
            vec![(1, 2), (6, 7)]
        );
    }

    #[test]
    fn test_coalesce_ranges_no_bridge_covered_in_gap() {
        // Lines 1,2,4,5 uncovered; line 3 is instrumentable (covered) → no bridge
        assert_eq!(
            coalesce_ranges(&[1, 2, 4, 5], &[1, 2, 3, 4, 5]),
            vec![(1, 2), (4, 5)]
        );
    }

    #[test]
    fn test_coalesce_ranges_mixed() {
        // Lines 1,2,4,5,10 uncovered; all are instrumentable, line 3 is not
        assert_eq!(
            coalesce_ranges(&[1, 2, 4, 5, 10], &[1, 2, 4, 5, 10]),
            vec![(1, 5), (10, 10)]
        );
    }

    // -- format_line_ranges tests -------------------------------------------

    #[test]
    fn test_format_line_ranges_empty() {
        assert_eq!(format_line_ranges(&[], &[]), "");
    }

    #[test]
    fn test_format_line_ranges_single() {
        assert_eq!(format_line_ranges(&[5], &[5]), "5");
    }

    #[test]
    fn test_format_line_ranges_consecutive() {
        assert_eq!(format_line_ranges(&[1, 2, 3], &[1, 2, 3]), "1-3");
    }

    #[test]
    fn test_format_line_ranges_mixed() {
        // All lines instrumentable → no bridging across gaps
        assert_eq!(
            format_line_ranges(&[1, 3, 4, 5, 10], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            "1, 3-5, 10"
        );
    }

    #[test]
    fn test_format_line_ranges_bridges_gap() {
        // Lines 1,2,4,5 uncovered; line 3 not instrumentable → "1-5"
        assert_eq!(format_line_ranges(&[1, 2, 4, 5], &[1, 2, 4, 5]), "1-5");
    }

    // -- format_line_ranges_linked tests ------------------------------------

    #[test]
    fn test_format_line_ranges_linked_empty() {
        assert_eq!(
            format_line_ranges_linked(&[], &[], "abc123", "src/foo.rs"),
            ""
        );
    }

    #[test]
    fn test_format_line_ranges_linked_single() {
        assert_eq!(
            format_line_ranges_linked(&[5], &[5], "abc123", "src/foo.rs"),
            "[5](../blob/abc123/src/foo.rs#L5)"
        );
    }

    #[test]
    fn test_format_line_ranges_linked_consecutive() {
        assert_eq!(
            format_line_ranges_linked(&[1, 2, 3], &[1, 2, 3], "abc123", "src/foo.rs"),
            "[1-3](../blob/abc123/src/foo.rs#L1-L3)"
        );
    }

    #[test]
    fn test_format_line_ranges_linked_mixed() {
        assert_eq!(
            format_line_ranges_linked(
                &[1, 3, 4, 5, 10],
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                "abc123",
                "src/foo.rs"
            ),
            "[1](../blob/abc123/src/foo.rs#L1), [3-5](../blob/abc123/src/foo.rs#L3-L5), [10](../blob/abc123/src/foo.rs#L10)"
        );
    }

    #[test]
    fn test_format_line_ranges_linked_bridges_gap() {
        assert_eq!(
            format_line_ranges_linked(&[1, 2, 4, 5], &[1, 2, 4, 5], "abc123", "src/foo.rs"),
            "[1-5](../blob/abc123/src/foo.rs#L1-L5)"
        );
    }

    #[test]
    fn test_format_markdown_all_covered() {
        let report = DiffCoverageReport {
            diff_files: 1,
            diff_lines: 10,
            files: vec![],
            total_covered: 10,
            total_instrumentable: 10,
            total_rate: Some(0.85),
            file_rates: HashMap::new(),
            sha: Some("abc1234def".to_string()),
        };
        let body = report.format(&MarkdownFormatter);
        assert!(body.contains("Diff Coverage: 100.0%"));
        assert!(body.contains("All diff lines are covered!"));
        assert!(body.contains("85.0%"));
        assert!(body.contains("[covrs](https://github.com/scttnlsn/covrs)"));
        assert!(body.contains("abc1234"));
    }

    #[test]
    fn test_format_markdown_with_misses() {
        let report = DiffCoverageReport {
            diff_files: 1,
            diff_lines: 5,
            files: vec![FileDiffCoverage {
                path: "src/foo.rs".to_string(),
                covered_lines: vec![1, 2, 3],
                missed_lines: vec![5, 6],
            }],
            total_covered: 3,
            total_instrumentable: 5,
            total_rate: None,
            file_rates: HashMap::from([("src/foo.rs".to_string(), 0.75)]),
            sha: None,
        };
        let body = report.format(&MarkdownFormatter);
        assert!(body.contains("60.0%"));
        assert!(body.contains("src/foo.rs"));
        assert!(body.contains("5-6"));
        assert!(body.contains("Missed lines"));
        // Total file coverage column
        assert!(body.contains("75%"));
    }

    #[test]
    fn test_format_markdown_with_misses_linked() {
        let report = DiffCoverageReport {
            diff_files: 1,
            diff_lines: 5,
            files: vec![FileDiffCoverage {
                path: "src/foo.rs".to_string(),
                covered_lines: vec![1, 2, 3],
                missed_lines: vec![5, 6],
            }],
            total_covered: 3,
            total_instrumentable: 5,
            total_rate: None,
            file_rates: HashMap::new(),
            sha: Some("abc1234def".to_string()),
        };
        let body = report.format(&MarkdownFormatter);
        assert!(body.contains("[5-6](../blob/abc1234def/src/foo.rs#L5-L6)"));
    }

    #[test]
    fn test_format_with_trait() {
        let report = DiffCoverageReport {
            diff_files: 1,
            diff_lines: 5,
            files: vec![],
            total_covered: 5,
            total_instrumentable: 5,
            total_rate: None,
            file_rates: HashMap::new(),
            sha: None,
        };

        // Test using the trait directly
        let text = report.format(&TextFormatter);
        assert!(text.contains("Diff coverage: 100.0%"));

        let md = report.format(&MarkdownFormatter);
        assert!(md.contains("Diff Coverage: 100.0%"));
    }
}
