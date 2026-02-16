//! Uniform in-memory representation of coverage data, independent of any
//! specific format. Parsers produce a `CoverageData` which is then inserted
//! into the SQLite store.

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
