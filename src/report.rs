//! Output formatting for diff coverage results.

use std::collections::HashMap;
use std::fmt::Write;

use crate::db::FileDiffCoverage;

/// Aggregated diff coverage data, ready to be formatted as text or markdown.
pub struct DiffCoverageReport {
    /// Number of added lines per file from the diff.
    pub diff_files: usize,
    /// Total number of added lines across all files.
    pub diff_lines: usize,
    /// Per-file coverage detail (only files with at least one instrumentable line).
    pub files: Vec<FileDiffCoverage>,
    /// Total instrumentable diff lines that are covered.
    pub total_covered: u64,
    /// Total instrumentable diff lines.
    pub total_instrumentable: u64,
    /// Overall repo line coverage rate (if available).
    pub repo_line_rate: Option<f64>,
    /// Commit SHA to display.
    pub sha: Option<String>,
}

impl DiffCoverageReport {
    /// Format as plain text.
    pub fn format_text(&self) -> String {
        let mut out = String::new();

        if self.diff_files == 0 {
            out.push_str("No added lines found in diff.\n");
            return out;
        }

        if self.total_instrumentable == 0 {
            writeln!(
                out,
                "{} lines added across {} files — none are instrumentable.",
                self.diff_lines, self.diff_files
            )
            .unwrap();
            return out;
        }

        let rate = self.total_covered as f64 / self.total_instrumentable as f64 * 100.0;
        writeln!(
            out,
            "Patch coverage: {:.1}% ({}/{} lines covered)",
            rate, self.total_covered, self.total_instrumentable
        )
        .unwrap();

        let files_with_misses: Vec<_> = self
            .files
            .iter()
            .filter(|f| !f.missed_lines.is_empty())
            .collect();
        if !files_with_misses.is_empty() {
            out.push('\n');
            for f in &files_with_misses {
                let file_total = f.total();
                let file_covered = f.covered_lines.len();
                let file_rate = if file_total > 0 {
                    file_covered as f64 / file_total as f64 * 100.0
                } else {
                    100.0
                };
                writeln!(
                    out,
                    "  {}  {}/{} ({:.1}%)  missed: {}",
                    f.path,
                    file_covered,
                    file_total,
                    file_rate,
                    format_line_ranges(&f.missed_lines),
                )
                .unwrap();
            }
        }

        if let Some(rate) = self.repo_line_rate {
            out.push('\n');
            writeln!(out, "Full project coverage: {:.1}%", rate * 100.0).unwrap();
        }

        out
    }

    /// Format as markdown.
    pub fn format_markdown(&self) -> String {
        let mut md = String::new();

        let patch_rate = if self.total_instrumentable > 0 {
            self.total_covered as f64 / self.total_instrumentable as f64 * 100.0
        } else {
            100.0
        };

        md.push_str(&format!("## Patch Coverage: {:.1}%\n\n", patch_rate));

        md.push_str(&format!(
            "**{}** of **{}** patch lines covered",
            self.total_covered, self.total_instrumentable
        ));
        if let Some(ref sha) = self.sha {
            let short_sha = if sha.len() > 7 { &sha[..7] } else { sha };
            md.push_str(&format!(" ({})", short_sha));
        }
        md.push('\n');

        let files_with_misses: Vec<&FileDiffCoverage> = self
            .files
            .iter()
            .filter(|f| !f.missed_lines.is_empty())
            .collect();

        if files_with_misses.is_empty() {
            md.push_str("\nAll patch lines are covered! 🎉\n");
        } else {
            md.push_str("\n| File | Missed | Patch | \n");
            md.push_str("|:-----|-------:|------:|\n");

            for f in &files_with_misses {
                let file_total = f.total();
                let file_covered = f.covered_lines.len();
                let file_rate = if file_total > 0 {
                    file_covered as f64 / file_total as f64 * 100.0
                } else {
                    100.0
                };
                md.push_str(&format!(
                    "| `{}` | {} | {:.0}% |\n",
                    f.path,
                    f.missed_lines.len(),
                    file_rate,
                ));
            }

            md.push_str("\n<details>\n<summary>Missed lines</summary>\n\n");

            for f in &files_with_misses {
                md.push_str(&format!(
                    "**`{}`**: {}\n\n",
                    f.path,
                    format_line_ranges(&f.missed_lines),
                ));
            }

            md.push_str("</details>\n");
        }

        md.push('\n');
        if let Some(rate) = self.repo_line_rate {
            md.push_str(&format!(
                "<sub>Full project coverage: **{:.1}%**</sub>\n",
                rate * 100.0
            ));
        }
        md.push_str("<sub>[covrs](https://github.com/scttnlsn/covrs)</sub>\n");

        md
    }
}

/// Build a [`DiffCoverageReport`] from parsed diff lines and a database connection.
pub fn build_report(
    conn: &rusqlite::Connection,
    diff_lines: &HashMap<String, Vec<u32>>,
    sha: Option<String>,
) -> anyhow::Result<DiffCoverageReport> {
    let diff_files = diff_lines.len();
    let diff_line_count: usize = diff_lines.values().map(|v| v.len()).sum();

    let (files, total_covered, total_instrumentable) = if diff_lines.is_empty() {
        (vec![], 0, 0)
    } else {
        crate::db::diff_coverage_detail(conn, diff_lines)?
    };

    let repo_line_rate = crate::db::get_overall_line_rate(conn)?;

    Ok(DiffCoverageReport {
        diff_files,
        diff_lines: diff_line_count,
        files,
        total_covered,
        total_instrumentable,
        repo_line_rate,
        sha,
    })
}

/// Format line numbers into compact range notation, e.g. "1, 3-5, 8".
pub fn format_line_ranges(lines: &[u32]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut ranges: Vec<String> = Vec::new();
    let mut start = lines[0];
    let mut end = lines[0];

    for &line in &lines[1..] {
        if line == end + 1 {
            end = line;
        } else {
            if start == end {
                ranges.push(format!("{}", start));
            } else {
                ranges.push(format!("{}-{}", start, end));
            }
            start = line;
            end = line;
        }
    }

    if start == end {
        ranges.push(format!("{}", start));
    } else {
        ranges.push(format!("{}-{}", start, end));
    }

    ranges.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_line_ranges_empty() {
        assert_eq!(format_line_ranges(&[]), "");
    }

    #[test]
    fn test_format_line_ranges_single() {
        assert_eq!(format_line_ranges(&[5]), "5");
    }

    #[test]
    fn test_format_line_ranges_consecutive() {
        assert_eq!(format_line_ranges(&[1, 2, 3]), "1-3");
    }

    #[test]
    fn test_format_line_ranges_mixed() {
        assert_eq!(format_line_ranges(&[1, 3, 4, 5, 10]), "1, 3-5, 10");
    }

    #[test]
    fn test_format_markdown_all_covered() {
        let report = DiffCoverageReport {
            diff_files: 1,
            diff_lines: 10,
            files: vec![],
            total_covered: 10,
            total_instrumentable: 10,
            repo_line_rate: Some(0.85),
            sha: Some("abc1234def".to_string()),
        };
        let body = report.format_markdown();
        assert!(body.contains("Patch Coverage: 100.0%"));
        assert!(body.contains("All patch lines are covered!"));
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
            repo_line_rate: None,
            sha: None,
        };
        let body = report.format_markdown();
        assert!(body.contains("60.0%"));
        assert!(body.contains("src/foo.rs"));
        assert!(body.contains("5-6"));
        assert!(body.contains("Missed lines"));
    }
}
