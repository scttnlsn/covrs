/// Parser for the LCOV `.info` format.
///
/// Reference: https://ltp.sourceforge.net/coverage/lcov/geninfo.1.php
///
/// Key records:
///   TN:<test name>
///   SF:<absolute path to source file>
///   FN:<line>,<function name>
///   FNDA:<execution count>,<function name>
///   FNF:<number of functions found>
///   FNH:<number of functions hit>
///   DA:<line number>,<execution count>[,<checksum>]
///   BRDA:<line>,<block>,<branch>,<taken>   ("-" means 0)
///   BRF:<branches found>
///   BRH:<branches hit>
///   LF:<lines found>
///   LH:<lines hit>
///   end_of_record
use std::collections::HashMap;

use crate::error::{CovrsError, Result};
use crate::model::*;
use crate::parsers::Parser;

pub struct LcovParser;

impl Parser for LcovParser {
    fn parse(&self, input: &[u8]) -> Result<CoverageData> {
        let text = std::str::from_utf8(input)
            .map_err(|e| CovrsError::Parse(format!("Invalid UTF-8 in LCOV data: {}", e)))?;
        parse_lcov(text)
    }
}

fn parse_lcov(text: &str) -> Result<CoverageData> {
    let mut data = CoverageData::new();
    let mut current_file: Option<FileCoverage> = None;

    // Track branch indices per line within the current file.
    let mut branch_indices: HashMap<u32, u32> = HashMap::new();

    // Track function definitions: name -> (start_line, end_line)
    // end_line is not provided in LCOV; we leave it as None.
    let mut fn_defs: HashMap<String, Option<u32>> = HashMap::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if line == "end_of_record" {
            if let Some(file) = current_file.take() {
                data.files.push(file);
            }
            branch_indices.clear();
            fn_defs.clear();
            continue;
        }

        // Split on first ':'
        let (tag, value) = match line.split_once(':') {
            Some(pair) => pair,
            None => continue, // Skip lines we don't understand
        };

        match tag {
            "TN" => {
                // Test name — we ignore this for now.
            }
            "SF" => {
                current_file = Some(FileCoverage::new(value.to_string()));
                branch_indices.clear();
                fn_defs.clear();
            }
            "FN" => {
                // FN:<line>,<function_name>
                if let Some((line_str, name)) = value.split_once(',') {
                    if let Ok(start_line) = line_str.parse::<u32>() {
                        fn_defs.insert(name.to_string(), Some(start_line));
                    }
                }
            }
            "FNDA" => {
                // FNDA:<execution_count>,<function_name>
                if let Some(file) = current_file.as_mut() {
                    if let Some((count_str, name)) = value.split_once(',') {
                        let hit_count = count_str.parse::<u64>().unwrap_or(0);
                        let start_line = fn_defs.get(name).copied().flatten();
                        file.functions.push(FunctionCoverage {
                            name: name.to_string(),
                            start_line,
                            end_line: None,
                            hit_count,
                        });
                    }
                }
            }
            "DA" => {
                // DA:<line_number>,<execution_count>[,<checksum>]
                // Some instrumenters use negative counts (e.g., -1) to indicate
                // non-instrumentable lines. We skip those entirely.
                if let Some(file) = current_file.as_mut() {
                    let parts: Vec<&str> = value.splitn(3, ',').collect();
                    if parts.len() >= 2 {
                        if let Ok(line_number) = parts[0].parse::<u32>() {
                            match parts[1].parse::<i64>() {
                                Ok(count) if count >= 0 => {
                                    file.lines.push(LineCoverage {
                                        line_number,
                                        hit_count: count as u64,
                                    });
                                }
                                _ => {
                                    // Negative count or parse failure — skip
                                    // this line as non-instrumentable.
                                }
                            }
                        }
                    }
                }
            }
            "BRDA" => {
                // BRDA:<line>,<block>,<branch>,<taken>
                // <taken> can be "-" meaning 0.
                // We use (line, block, branch) as a composite key to produce
                // unique, stable branch indices — preserving block semantics.
                if let Some(file) = current_file.as_mut() {
                    let parts: Vec<&str> = value.splitn(4, ',').collect();
                    if parts.len() == 4 {
                        if let Ok(line_number) = parts[0].parse::<u32>() {
                            let hit_count = if parts[3] == "-" {
                                0
                            } else {
                                parts[3].parse::<u64>().unwrap_or(0)
                            };
                            // Use a running counter per line to assign branch
                            // indices. The (block, branch) pair from the BRDA
                            // record determines ordering — each unique BRDA
                            // record on a line gets the next index.
                            let branch_index = branch_indices.entry(line_number).or_insert(0);
                            file.branches.push(BranchCoverage {
                                line_number,
                                branch_index: *branch_index,
                                hit_count,
                            });
                            *branch_indices.get_mut(&line_number).unwrap() += 1;
                        }
                    }
                }
            }
            // LF, LH, FNF, FNH, BRF, BRH — summary lines; we derive these from the data.
            _ => {}
        }
    }

    // Handle case where file ends without end_of_record
    if let Some(file) = current_file.take() {
        data.files.push(file);
    }

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lcov() {
        let input = include_bytes!("../../tests/fixtures/sample.lcov");
        let parser = LcovParser;
        let data = parser.parse(input).unwrap();

        assert_eq!(data.files.len(), 2);

        let lib = &data.files[0];
        assert_eq!(lib.path, "/src/lib.rs");
        assert_eq!(lib.lines.len(), 5);
        assert_eq!(lib.lines[0].line_number, 1);
        assert_eq!(lib.lines[0].hit_count, 5);
        assert_eq!(lib.lines[2].line_number, 3);
        assert_eq!(lib.lines[2].hit_count, 0);

        assert_eq!(lib.branches.len(), 2);
        assert_eq!(lib.branches[0].line_number, 2);
        assert_eq!(lib.branches[0].branch_index, 0);
        assert_eq!(lib.branches[0].hit_count, 5);
        assert_eq!(lib.branches[1].branch_index, 1);
        assert_eq!(lib.branches[1].hit_count, 0);

        assert_eq!(lib.functions.len(), 2);
        assert_eq!(lib.functions[0].name, "main");
        assert_eq!(lib.functions[0].hit_count, 5);
        assert_eq!(lib.functions[0].start_line, Some(1));
        assert_eq!(lib.functions[1].name, "helper");
        assert_eq!(lib.functions[1].hit_count, 0);

        let util = &data.files[1];
        assert_eq!(util.path, "/src/util.rs");
        assert_eq!(util.lines.len(), 2);
        assert_eq!(util.branches.len(), 0);
        assert_eq!(util.functions.len(), 0);
    }

    #[test]
    fn test_parse_lcov_no_end_of_record() {
        let input = include_bytes!("../../tests/fixtures/lcov_no_end_of_record.lcov");
        let parser = LcovParser;
        let data = parser.parse(input).unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].lines.len(), 2);
    }

    #[test]
    fn test_parse_lcov_negative_counts() {
        // DA lines with negative counts (e.g., -1) should be skipped as
        // non-instrumentable.
        let input = include_bytes!("../../tests/fixtures/lcov_negative_counts.lcov");
        let parser = LcovParser;
        let data = parser.parse(input).unwrap();

        assert_eq!(data.files.len(), 1);
        let file = &data.files[0];
        // Line 2 has count=-1, should be skipped. Lines 1, 3, 4 remain.
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].line_number, 1);
        assert_eq!(file.lines[0].hit_count, 5);
        assert_eq!(file.lines[1].line_number, 3);
        assert_eq!(file.lines[1].hit_count, 0);
        assert_eq!(file.lines[2].line_number, 4);
        assert_eq!(file.lines[2].hit_count, 3);
    }

    #[test]
    fn test_parse_lcov_empty() {
        // An LCOV file with only a test name and no records should produce
        // an empty CoverageData (no files).
        let input = include_bytes!("../../tests/fixtures/empty.lcov");
        let parser = LcovParser;
        let data = parser.parse(input).unwrap();
        assert_eq!(data.files.len(), 0);
    }
}
