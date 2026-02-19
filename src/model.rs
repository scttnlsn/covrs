//! Uniform in-memory representation of coverage data, independent of any
//! specific format. Parsers produce a `CoverageData` which is then inserted
//! into the SQLite store.

/// Compute a coverage rate, returning 0.0 when the total is zero.
#[must_use]
pub fn rate(covered: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        covered as f64 / total as f64
    }
}

/// A single line that was instrumentable.
#[derive(Debug, Clone)]
pub struct LineCoverage {
    pub line_number: u32,
    pub hit_count: u64,
}

/// A single branch arm on a given line.
#[derive(Debug, Clone)]
pub struct BranchCoverage {
    pub line_number: u32,
    pub branch_index: u32,
    pub hit_count: u64,
}

/// A function/method that was instrumentable.
#[derive(Debug, Clone)]
pub struct FunctionCoverage {
    pub name: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub hit_count: u64,
}

/// Coverage data for a single source file.
#[derive(Debug, Clone, Default)]
pub struct FileCoverage {
    pub path: String,
    pub lines: Vec<LineCoverage>,
    pub branches: Vec<BranchCoverage>,
    pub functions: Vec<FunctionCoverage>,
}

impl FileCoverage {
    pub fn new(path: String) -> Self {
        Self {
            path,
            ..Default::default()
        }
    }
}

/// The complete result of parsing a single coverage file.
#[derive(Debug, Clone, Default)]
pub struct CoverageData {
    pub files: Vec<FileCoverage>,
}

impl CoverageData {
    pub fn new() -> Self {
        Self::default()
    }
}

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
    #[must_use]
    pub fn line_rate(&self) -> f64 {
        rate(self.covered_lines, self.total_lines)
    }

    #[must_use]
    pub fn branch_rate(&self) -> f64 {
        rate(self.covered_branches, self.total_branches)
    }

    #[must_use]
    pub fn function_rate(&self) -> f64 {
        rate(self.covered_functions, self.total_functions)
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
    #[must_use]
    pub fn line_rate(&self) -> f64 {
        rate(self.covered_lines, self.total_lines)
    }
}

/// Line-level detail for a source file.
#[derive(Debug)]
pub struct LineDetail {
    pub line_number: u32,
    pub hit_count: u64,
}

/// Metadata for a stored report.
#[derive(Debug)]
pub struct ReportInfo {
    pub name: String,
    pub format: String,
    pub created_at: String,
}

/// Per-file diff coverage detail.
#[derive(Debug)]
pub struct FileDiffCoverage {
    pub path: String,
    /// Diff lines that are instrumentable and covered.
    pub covered_lines: Vec<u32>,
    /// Diff lines that are instrumentable and NOT covered.
    pub missed_lines: Vec<u32>,
}

impl FileDiffCoverage {
    #[must_use]
    pub fn total(&self) -> usize {
        self.covered_lines.len() + self.missed_lines.len()
    }

    #[must_use]
    pub fn rate(&self) -> f64 {
        rate(self.covered_lines.len() as u64, self.total() as u64)
    }
}

/// A single annotation to attach to a GitHub check run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Annotation {
    /// Source file path relative to the repo root.
    pub path: String,
    /// Start line of the annotation range.
    pub start_line: u32,
    /// End line of the annotation range.
    pub end_line: u32,
    /// Annotation message.
    pub message: String,
}
