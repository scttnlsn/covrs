use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::db;
use crate::parsers::{self, Format};

/// Read a coverage file, auto-detect its format (or use the override),
/// parse it, and insert into the database.
/// Returns (report_id, format, actual_report_name).
pub fn ingest(
    conn: &mut Connection,
    file_path: &Path,
    format_override: Option<&str>,
    report_name: Option<&str>,
    overwrite: bool,
) -> Result<(i64, Format, String)> {
    let content = std::fs::read(file_path)
        .with_context(|| format!("Failed to read {}", file_path.display()))?;

    // Get the right parser — explicit override or auto-detect
    let parser = if let Some(fmt_str) = format_override {
        let format = fmt_str.parse::<Format>()?;
        parsers::for_format(format)
    } else {
        parsers::detect(file_path, &content)
            .ok_or_else(|| anyhow::anyhow!("Unknown coverage format"))?
    };

    let format = parser.format();
    let data = parser.parse(&content)?;

    // Warn on empty coverage data
    if data.files.is_empty() {
        eprintln!(
            "Warning: coverage file '{}' contains no source files",
            file_path.display()
        );
    }

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

    let report_id = db::insert_coverage(
        conn,
        &name,
        &format.to_string(),
        source_file_str,
        &data,
        overwrite,
    )?;

    Ok((report_id, format, name))
}
