/// Parser for Istanbul / NYC `coverage-final.json` format.
///
/// Reference: https://github.com/istanbuljs/istanbuljs
///
/// The format is a JSON object keyed by file path. Each value contains:
///   - `statementMap`: `{ "0": { "start": { "line": 1, "column": 0 }, "end": { "line": 1, "column": 30 } }, ... }`
///   - `s`:            `{ "0": 5, "1": 0, ... }` — hit counts per statement
///   - `branchMap`:    `{ "0": { "loc": ..., "type": "if", "locations": [...] }, ... }`
///   - `b`:            `{ "0": [5, 0], ... }` — hit counts per branch arm
///   - `fnMap`:        `{ "0": { "name": "foo", "decl": { "start": { "line": 1 }, ... }, "loc": ... }, ... }`
///   - `f`:            `{ "0": 3, ... }` — hit counts per function
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::{CoverageParser, Format};
use crate::model::*;

/// Istanbul / NYC JSON parser.
pub struct IstanbulParser;

impl CoverageParser for IstanbulParser {
    fn format(&self) -> Format {
        Format::Istanbul
    }

    fn can_parse(&self, path: &Path, content: &[u8]) -> bool {
        // Extension-based: common Istanbul output filenames
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let name = name.to_lowercase();
            if name == "coverage-final.json" {
                return true;
            }
        }

        // Content-based: JSON object whose first value contains "statementMap"
        let head = super::sniff_head(content);
        looks_like_istanbul(&head)
    }

    fn parse_streaming(
        &self,
        reader: &mut dyn BufRead,
        emit: &mut dyn FnMut(FileCoverage) -> Result<()>,
    ) -> Result<()> {
        parse_streaming_reader(reader, emit)
    }
}

/// Parse Istanbul JSON from raw bytes.
pub fn parse(input: &[u8]) -> Result<CoverageData> {
    let mut data = CoverageData::new();
    parse_streaming_reader(&mut &*input, &mut |file| {
        data.files.push(file);
        Ok(())
    })?;
    Ok(data)
}

/// Content-based detection: a JSON object where at least one value
/// contains `"statementMap"` and `"fnMap"`.
fn looks_like_istanbul(head: &str) -> bool {
    let trimmed = head.trim();
    // Must start with '{' (JSON object)
    if !trimmed.starts_with('{') {
        return false;
    }
    // Look for Istanbul-specific keys
    trimmed.contains("\"statementMap\"") && trimmed.contains("\"fnMap\"")
}

/// Streaming parser — deserializes the top-level JSON object entry by
/// entry using a serde `MapAccess` visitor so only one file entry is in
/// memory at a time.
fn parse_streaming_reader(
    reader: &mut dyn BufRead,
    emit: &mut dyn FnMut(FileCoverage) -> Result<()>,
) -> Result<()> {
    // Peek at the first byte (skipping whitespace) to handle empty input.
    let buf = reader.fill_buf().context("Failed to read Istanbul JSON")?;
    if buf.is_empty() || buf.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(());
    }

    let mut deser = serde_json::Deserializer::from_reader(reader);

    // Use a visitor that walks the top-level object key by key,
    // converting each value into a FileCoverage before moving on.
    let mut emit_err: Result<()> = Ok(());
    let visitor = IstanbulVisitor {
        emit: &mut |fc| {
            emit(fc).map_err(|e| {
                emit_err = Err(anyhow::anyhow!("{e}"));
                serde::de::Error::custom(e.to_string())
            })
        },
    };
    match serde::Deserializer::deserialize_map(&mut deser, visitor) {
        Ok(()) => emit_err,
        Err(e) => {
            // If the error originated from `emit`, return the original.
            emit_err?;
            Err(e).context("Invalid JSON in Istanbul file")
        }
    }
}

/// Serde visitor that iterates over the top-level `{ path: entry }` map,
/// deserializing one `Value` per entry and emitting a `FileCoverage`.
struct IstanbulVisitor<'a> {
    emit: &'a mut dyn FnMut(FileCoverage) -> std::result::Result<(), serde::de::value::Error>,
}

impl<'de, 'a> serde::de::Visitor<'de> for IstanbulVisitor<'a> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an Istanbul JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        while let Some(file_path) = map.next_key::<String>()? {
            let entry: Value = map.next_value()?;
            let file_cov =
                parse_file_entry(&file_path, &entry).map_err(serde::de::Error::custom)?;
            (self.emit)(file_cov).map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

/// Parse a single file entry from the Istanbul JSON.
fn parse_file_entry(file_path: &str, entry: &Value) -> Result<FileCoverage> {
    let mut file = FileCoverage::new(file_path.to_string());

    // ── Statements → lines ────────────────────────────────────────
    parse_statements(entry, &mut file);

    // ── Branches ──────────────────────────────────────────────────
    parse_branches(entry, &mut file);

    // ── Functions ─────────────────────────────────────────────────
    parse_functions(entry, &mut file);

    // Sort for deterministic output.
    file.lines.sort_by_key(|l| l.line_number);
    file.branches
        .sort_by_key(|b| (b.line_number, b.branch_index));
    file.functions.sort_by_key(|f| f.start_line);

    Ok(file)
}

/// Extract per-line coverage from `statementMap` + `s`.
///
/// `statementMap` maps string indices to `{ start: { line, column }, end: { line, column } }`.
/// `s` maps the same indices to hit counts.
///
/// Multiple statements can map to the same line; we take the maximum
/// hit count for each line.
fn parse_statements(entry: &Value, file: &mut FileCoverage) {
    let stmt_map = match entry.get("statementMap").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };
    let s = match entry.get("s").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    let mut line_hits: HashMap<u32, u64> = HashMap::new();

    for (idx, loc) in stmt_map {
        let line = match loc
            .get("start")
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
        {
            Some(l) => l as u32,
            None => continue,
        };

        let count = s.get(idx.as_str()).and_then(|v| v.as_u64()).unwrap_or(0);

        line_hits
            .entry(line)
            .and_modify(|e| *e = (*e).max(count))
            .or_insert(count);
    }

    for (line_number, hit_count) in line_hits {
        file.lines.push(LineCoverage {
            line_number,
            hit_count,
        });
    }
}

/// Extract branch coverage from `branchMap` + `b`.
///
/// `branchMap` maps string indices to `{ type, locations: [{ start: { line } }, ...] }`.
/// `b` maps the same indices to arrays of hit counts (one per branch arm).
fn parse_branches(entry: &Value, file: &mut FileCoverage) {
    let branch_map = match entry.get("branchMap").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };
    let b = match entry.get("b").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    // Track branch indices per line to assign sequential indices.
    let mut line_branch_idx: HashMap<u32, u32> = HashMap::new();

    for (idx, branch_info) in branch_map {
        // Get the line number from the branch location (use the top-level
        // `loc.start.line` if available, otherwise the first location).
        let line = branch_info
            .get("loc")
            .and_then(|loc| loc.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .or_else(|| {
                branch_info
                    .get("locations")
                    .and_then(|locs| locs.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|loc| loc.get("start"))
                    .and_then(|s| s.get("line"))
                    .and_then(|l| l.as_u64())
            });

        let line = match line {
            Some(l) => l as u32,
            None => continue,
        };

        let counts = match b.get(idx.as_str()).and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for count_val in counts {
            let hit_count = count_val.as_u64().unwrap_or(0);
            let branch_index = line_branch_idx.entry(line).or_insert(0);
            file.branches.push(BranchCoverage {
                line_number: line,
                branch_index: *branch_index,
                hit_count,
            });
            *branch_index += 1;
        }
    }
}

/// Extract function coverage from `fnMap` + `f`.
///
/// `fnMap` maps string indices to `{ name, decl: { start: { line } }, loc: { start: { line }, end: { line } } }`.
/// `f` maps the same indices to hit counts.
fn parse_functions(entry: &Value, file: &mut FileCoverage) {
    let fn_map = match entry.get("fnMap").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };
    let f = match entry.get("f").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for (idx, fn_info) in fn_map {
        let name = fn_info
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("(anonymous)")
            .to_string();

        // `decl.start.line` is the declaration line; `loc` is the body.
        let start_line = fn_info
            .get("decl")
            .or_else(|| fn_info.get("loc"))
            .and_then(|loc| loc.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .map(|l| l as u32);

        let end_line = fn_info
            .get("loc")
            .and_then(|loc| loc.get("end"))
            .and_then(|e| e.get("line"))
            .and_then(|l| l.as_u64())
            .map(|l| l as u32);

        let hit_count = f.get(idx.as_str()).and_then(|v| v.as_u64()).unwrap_or(0);

        file.functions.push(FunctionCoverage {
            name,
            start_line,
            end_line,
            hit_count,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_istanbul() {
        let input = include_bytes!("../../tests/fixtures/sample_istanbul.json");
        let data = parse(input).unwrap();

        assert_eq!(data.files.len(), 2);

        let lib = data
            .files
            .iter()
            .find(|f| f.path.ends_with("lib.js"))
            .unwrap();
        assert_eq!(lib.lines.len(), 5);
        // Lines should be sorted
        assert_eq!(lib.lines[0].line_number, 1);
        assert_eq!(lib.lines[0].hit_count, 5);
        assert_eq!(lib.lines[2].line_number, 3);
        assert_eq!(lib.lines[2].hit_count, 0);

        assert_eq!(lib.branches.len(), 2);
        assert_eq!(lib.branches[0].hit_count + lib.branches[1].hit_count, 5); // one arm hit, one not

        assert_eq!(lib.functions.len(), 2);
        let main_fn = lib.functions.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(main_fn.hit_count, 5);
        assert_eq!(main_fn.start_line, Some(1));

        let util = data
            .files
            .iter()
            .find(|f| f.path.ends_with("util.js"))
            .unwrap();
        assert_eq!(util.lines.len(), 2);
        assert_eq!(util.branches.len(), 0);
        assert_eq!(util.functions.len(), 0);
    }

    #[test]
    fn test_parse_istanbul_empty_object() {
        let input = b"{}";
        let data = parse(input).unwrap();
        assert_eq!(data.files.len(), 0);
    }

    #[test]
    fn test_parse_istanbul_empty_file() {
        let input = include_bytes!("../../tests/fixtures/empty_istanbul.json");
        let data = parse(input).unwrap();
        assert_eq!(data.files.len(), 0);
    }

    #[test]
    fn test_parse_istanbul_multiple_statements_same_line() {
        // Two statements on the same line — take the max hit count.
        let input = r#"{
            "/src/app.js": {
                "statementMap": {
                    "0": { "start": { "line": 1, "column": 0 }, "end": { "line": 1, "column": 10 } },
                    "1": { "start": { "line": 1, "column": 12 }, "end": { "line": 1, "column": 20 } }
                },
                "s": { "0": 3, "1": 7 },
                "branchMap": {},
                "b": {},
                "fnMap": {},
                "f": {}
            }
        }"#;
        let data = parse(input.as_bytes()).unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].lines.len(), 1);
        assert_eq!(data.files[0].lines[0].hit_count, 7); // max(3, 7)
    }

    #[test]
    fn test_looks_like_istanbul() {
        assert!(looks_like_istanbul(
            r#"{ "/src/lib.js": { "statementMap": {}, "fnMap": {} } }"#
        ));
        assert!(!looks_like_istanbul(r#"<?xml version="1.0"?>"#));
        assert!(!looks_like_istanbul(r#"SF:/src/lib.rs"#));
        assert!(!looks_like_istanbul(r#"{ "unrelated": true }"#));
        // "s" alone is too generic — require "fnMap"
        assert!(!looks_like_istanbul(
            r#"{ "statementMap": "x", "s": true }"#
        ));
    }

    #[test]
    fn test_can_parse_by_filename() {
        let parser = IstanbulParser;
        assert!(parser.can_parse(Path::new("coverage-final.json"), b""));
        assert!(parser.can_parse(Path::new("dir/coverage-final.json"), b""));
        assert!(!parser.can_parse(Path::new("coverage.json"), b""));
        assert!(!parser.can_parse(Path::new("data.json"), b""));
    }
}
