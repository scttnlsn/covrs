use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::db;
use crate::model::FileCoverage;
use crate::parsers::{self, Format};

/// Normalize a single file's path relative to the project root.
fn normalize_file_path(file: &mut FileCoverage, root: &Path) {
    let path = Path::new(&file.path);
    if path.is_absolute() && path.starts_with(root) {
        if let Ok(relative) = path.strip_prefix(root) {
            file.path = relative.to_string_lossy().into_owned();
        }
    }
}

/// How many bytes to read for format auto-detection.
const SNIFF_SIZE: usize = 4096;

/// Read a coverage file, auto-detect its format (or use the override),
/// parse it, and insert into the database.
///
/// The file is streamed through a buffered reader — only a small detection
/// buffer is read up front, then the parser reads incrementally. This keeps
/// memory usage independent of input file size.
///
/// When `root` is `Some`, absolute file paths in the coverage data are
/// made relative to the given root directory. Pass `None` to skip
/// normalization (paths are stored as-is from the coverage file).
///
/// Returns (report_id, format, actual_report_name).
pub fn ingest(
    conn: &mut Connection,
    file_path: &Path,
    format_override: Option<&str>,
    report_name: Option<&str>,
    overwrite: bool,
    root: Option<&Path>,
) -> Result<(i64, Format, String)> {
    let file =
        File::open(file_path).with_context(|| format!("Failed to open {}", file_path.display()))?;
    let mut reader = BufReader::new(file);

    // Get the right parser — explicit override or auto-detect.
    // For auto-detect we sniff the first SNIFF_SIZE bytes and then seek
    // back so the parser sees the complete stream.
    let parser = if let Some(fmt_str) = format_override {
        let format = fmt_str.parse::<Format>()?;
        parsers::for_format(format)
    } else {
        let mut head = vec![0u8; SNIFF_SIZE];
        let n = reader
            .read(&mut head)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        head.truncate(n);
        let detected = parsers::detect(file_path, &head)
            .ok_or_else(|| anyhow::anyhow!("Unknown coverage format"))?;
        reader
            .seek(std::io::SeekFrom::Start(0))
            .with_context(|| format!("Failed to seek {}", file_path.display()))?;
        detected
    };

    let format = parser.format();

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

    // Track whether any files were emitted so we can warn on empty input.
    let mut file_count: usize = 0;

    let report_id = db::insert_coverage_streaming(
        conn,
        &name,
        &format.to_string(),
        source_file_str,
        overwrite,
        |emit| {
            parser.parse_streaming(&mut reader, &mut |mut file_cov| {
                if let Some(root) = root {
                    normalize_file_path(&mut file_cov, root);
                }
                file_count += 1;
                emit(&file_cov)
            })
        },
    )?;

    if file_count == 0 {
        eprintln!(
            "Warning: coverage file '{}' contains no source files",
            file_path.display()
        );
    }

    Ok((report_id, format, name))
}
