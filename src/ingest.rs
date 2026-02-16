use std::path::Path;

use crate::db;
use crate::detect::{detect_format, Format};
use crate::error::{CovrsError, Result};
use crate::model::CoverageData;
use crate::parsers::cobertura::CoberturaParser;
use crate::parsers::lcov::LcovParser;
use crate::parsers::Parser;
use rusqlite::Connection;

/// Read a coverage file, auto-detect its format (or use the override),
/// parse it, and insert into the database.
/// Returns (report_id, detected_format, actual_report_name).
pub fn ingest(
    conn: &mut Connection,
    file_path: &Path,
    format_override: Option<&str>,
    report_name: Option<&str>,
) -> Result<(i64, Format, String)> {
    let content = std::fs::read(file_path)?;

    // Determine format
    let format = if let Some(fmt_str) = format_override {
        fmt_str.parse::<Format>()?
    } else {
        detect_format(file_path, &content).ok_or(CovrsError::UnknownFormat)?
    };

    // Parse
    let data = parse_with_format(format, &content)?;

    // Generate report name if not provided
    let name = match report_name {
        Some(n) => n.to_string(),
        None => file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string(),
    };

    let source_file_str = file_path.to_str();

    let report_id = db::insert_coverage(conn, &name, format.as_str(), source_file_str, &data)?;

    Ok((report_id, format, name))
}

fn parse_with_format(format: Format, content: &[u8]) -> Result<CoverageData> {
    match format {
        Format::Cobertura => CoberturaParser.parse(content),
        Format::Lcov => LcovParser.parse(content),
    }
}
