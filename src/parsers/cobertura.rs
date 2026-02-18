/// Parser for Cobertura XML coverage reports.
///
/// Cobertura XML structure:
///   <coverage>
///     <sources><source>...</source></sources>
///     <packages>
///       <package name="...">
///         <classes>
///           <class name="..." filename="..." line-rate="..." branch-rate="...">
///             <methods>
///               <method name="..." ... line-rate="...">
///                 <lines><line number="..." hits="..." .../></lines>
///               </method>
///             </methods>
///             <lines>
///               <line number="..." hits="..." branch="true|false"
///                     condition-coverage="50% (1/2)" />
///             </lines>
///           </class>
///         </classes>
///       </package>
///     </packages>
///   </coverage>
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use quick_xml::events::Event;
use regex::Regex;

/// Pre-compiled regex for condition-coverage attributes like "75% (3/4)".
static BRANCH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d+)/(\d+)\)").unwrap());

use super::{get_attr, CoverageParser, Format};
use crate::model::*;

/// Cobertura XML format parser.
pub struct CoberturaParser;

impl CoverageParser for CoberturaParser {
    fn format(&self) -> Format {
        Format::Cobertura
    }

    fn can_parse(&self, _path: &Path, content: &[u8]) -> bool {
        let head = super::sniff_head(content);
        super::looks_like_xml(&head) && head.contains("<coverage")
    }

    fn parse(&self, input: &[u8]) -> Result<CoverageData> {
        parse(input)
    }
}

/// Parse Cobertura XML coverage data from raw bytes.
pub fn parse(input: &[u8]) -> Result<CoverageData> {
    let mut reader = super::xml_reader(input);

    let mut data = CoverageData::new();
    let mut buf = Vec::new();

    // State tracking
    let mut current_file: Option<FileCoverage> = None;
    let mut in_method = false;
    let mut current_method_name: Option<String> = None;
    let mut method_hit: bool = false;
    let mut method_start_line: Option<u32> = None;
    let mut branch_indices: HashMap<u32, u32> = HashMap::new();
    let mut line_index_map: HashMap<u32, usize> = HashMap::new();

    // Source prefix from <source> elements
    let mut sources: Vec<String> = Vec::new();
    let mut in_source = false;

    let branch_re = &*BRANCH_RE;

    loop {
        let event = reader.read_event_into(&mut buf);
        let is_start_event = matches!(&event, Ok(Event::Start(_)));
        match event {
            Err(e) => return Err(super::xml_err(e, &reader)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                match e.name().as_ref() {
                    b"source" => {
                        // Only set in_source for Start events; self-closing
                        // <source/> (Empty) has no text content and no
                        // corresponding End event, so setting the flag would
                        // cause the next unrelated Text event to be captured.
                        if is_start_event {
                            in_source = true;
                        }
                    }
                    b"class" => {
                        if let Some(filename) = get_attr(e, b"filename") {
                            let path = resolve_source_path(&filename, &sources);
                            current_file = Some(FileCoverage::new(path));
                            branch_indices.clear();
                            line_index_map.clear();
                        }
                    }
                    b"method" => {
                        in_method = true;
                        current_method_name = get_attr(e, b"name");
                        method_hit = false;
                        method_start_line = None;
                    }
                    b"line" => {
                        if let Some(file) = current_file.as_mut() {
                            // Extract all needed attributes in a single pass
                            let mut number: Option<u32> = None;
                            let mut hits: u64 = 0;
                            let mut is_branch = false;
                            let mut cond_cov: Option<String> = None;

                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"number" => {
                                        number =
                                            attr.unescape_value().ok().and_then(|v| v.parse().ok());
                                    }
                                    b"hits" => {
                                        hits = attr
                                            .unescape_value()
                                            .ok()
                                            .and_then(|v| v.parse().ok())
                                            .unwrap_or(0);
                                    }
                                    b"branch" => {
                                        is_branch = attr
                                            .unescape_value()
                                            .ok()
                                            .map(|v| v == "true")
                                            .unwrap_or(false);
                                    }
                                    b"condition-coverage" => {
                                        cond_cov =
                                            attr.unescape_value().ok().map(|v| v.into_owned());
                                    }
                                    _ => {}
                                }
                            }

                            if let Some(line_number) = number {
                                let hit_count = hits;

                                // Always collect line coverage. Lines may appear both
                                // under <method><lines> and <class><lines>, or only in
                                // one of them depending on the generator. We deduplicate
                                // by keeping the max hit_count for each line number.
                                if let Some(&idx) = line_index_map.get(&line_number) {
                                    if hit_count > file.lines[idx].hit_count {
                                        file.lines[idx].hit_count = hit_count;
                                    }
                                } else {
                                    line_index_map.insert(line_number, file.lines.len());
                                    file.lines.push(LineCoverage {
                                        line_number,
                                        hit_count,
                                    });
                                }

                                // Track method start line and hit status
                                if in_method {
                                    if method_start_line.is_none() {
                                        method_start_line = Some(line_number);
                                    }
                                    if hit_count > 0 {
                                        method_hit = true;
                                    }
                                }

                                // Branch coverage — only process on first
                                // encounter of this line to avoid double-counting
                                // when the same line appears in both <method> and
                                // <class> blocks.
                                if is_branch && !branch_indices.contains_key(&line_number) {
                                    if let Some(cond) = cond_cov.as_deref() {
                                        if let Some(caps) = branch_re.captures(cond) {
                                            let covered: u32 = caps[1].parse().unwrap_or(0);
                                            let total: u32 = caps[2].parse().unwrap_or(0);

                                            for i in 0..total {
                                                // Cobertura's condition-coverage only tells
                                                // us how many branches were taken, not per-
                                                // branch execution counts. Use 1 for covered
                                                // arms and 0 for uncovered.
                                                let branch_hit: u64 =
                                                    if i < covered { 1 } else { 0 };
                                                let idx =
                                                    branch_indices.entry(line_number).or_insert(0);
                                                file.branches.push(BranchCoverage {
                                                    line_number,
                                                    branch_index: *idx,
                                                    hit_count: branch_hit,
                                                });
                                                *idx += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_source {
                    if let Ok(text) = e.unescape() {
                        sources.push(text.to_string());
                    }
                    in_source = false;
                }
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"source" => {
                    in_source = false;
                }
                b"class" => {
                    if let Some(file) = current_file.take() {
                        data.files.push(file);
                    }
                }
                b"method" => {
                    if in_method {
                        if let (Some(file), Some(name)) =
                            (current_file.as_mut(), current_method_name.take())
                        {
                            file.functions.push(FunctionCoverage {
                                name,
                                start_line: method_start_line,
                                end_line: None,
                                hit_count: if method_hit { 1 } else { 0 },
                            });
                        }
                        in_method = false;
                        method_start_line = None;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }

    // Handle unclosed file
    if let Some(file) = current_file.take() {
        data.files.push(file);
    }

    // Sort lines within each file by line number for consistent output,
    // since lines may have been collected from both <method> and <class> blocks.
    for file in &mut data.files {
        file.lines.sort_by_key(|l| l.line_number);
    }

    Ok(data)
}

/// Resolve a filename against the list of `<source>` prefixes.
///
/// - If the filename is already absolute, return it as-is.
/// - Otherwise, prepend the first non-empty source prefix.
/// - If no non-empty sources exist, return the filename unchanged.
fn resolve_source_path(filename: &str, sources: &[String]) -> String {
    if filename.starts_with('/') {
        return filename.to_string();
    }
    for source in sources {
        let base = source.trim_end_matches('/');
        if !base.is_empty() {
            return format!("{base}/{filename}");
        }
    }
    filename.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cobertura() {
        let input = include_bytes!("../../tests/fixtures/sample_cobertura.xml");
        let data = parse(input).unwrap();

        assert_eq!(data.files.len(), 2);

        let main = &data.files[0];
        assert_eq!(main.path, "/home/user/project/src/main.py");
        assert_eq!(main.lines.len(), 8);
        assert_eq!(main.lines[0].line_number, 1);
        assert_eq!(main.lines[0].hit_count, 1);
        assert_eq!(main.lines[2].line_number, 3);
        assert_eq!(main.lines[2].hit_count, 0);

        // Branch on line 8: 50% (1/2) → 2 branch arms, one hit one miss
        assert_eq!(main.branches.len(), 2);
        assert_eq!(main.branches[0].line_number, 8);
        assert_eq!(main.branches[0].hit_count, 1); // covered arm
        assert_eq!(main.branches[1].hit_count, 0); // uncovered arm

        // One method extracted
        assert_eq!(main.functions.len(), 1);
        assert_eq!(main.functions[0].name, "do_stuff");
        assert_eq!(main.functions[0].start_line, Some(5));
        assert_eq!(main.functions[0].hit_count, 1);

        let util = &data.files[1];
        assert_eq!(util.path, "/home/user/project/src/util.py");
        assert_eq!(util.lines.len(), 2);
        assert_eq!(util.branches.len(), 0);
    }

    #[test]
    fn test_parse_cobertura_branch_dedup() {
        // Branch line appears in both <method><lines> and <class><lines>.
        // We must not double-count the branch arms.
        let input = include_bytes!("../../tests/fixtures/cobertura_branch_in_method_and_class.xml");
        let data = parse(input).unwrap();

        assert_eq!(data.files.len(), 1);
        let file = &data.files[0];

        // Lines should be deduplicated: 4 unique lines, not 7
        assert_eq!(file.lines.len(), 4);

        // Branch on line 3: 50% (1/2) → exactly 2 arms, not 4
        assert_eq!(file.branches.len(), 2);
        assert_eq!(file.branches[0].line_number, 3);
        assert_eq!(file.branches[0].branch_index, 0);
        assert_eq!(file.branches[0].hit_count, 1); // covered arm
        assert_eq!(file.branches[1].line_number, 3);
        assert_eq!(file.branches[1].branch_index, 1);
        assert_eq!(file.branches[1].hit_count, 0); // uncovered arm
    }

    #[test]
    fn test_parse_cobertura_multiple_sources() {
        // First <source> is empty, second is the real prefix.
        let input = include_bytes!("../../tests/fixtures/cobertura_multiple_sources.xml");
        let data = parse(input).unwrap();

        assert_eq!(data.files.len(), 1);
        // Should use the first non-empty source as prefix, not the empty one.
        assert_eq!(data.files[0].path, "/home/user/project/src/app.py");
    }

    #[test]
    fn test_parse_cobertura_no_sources() {
        let input = include_bytes!("../../tests/fixtures/cobertura_no_sources.xml");
        let data = parse(input).unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].path, "src/f.rs");
    }

    #[test]
    fn test_parse_cobertura_empty() {
        // A valid Cobertura file with no classes should produce empty CoverageData.
        let input = include_bytes!("../../tests/fixtures/empty_cobertura.xml");
        let data = parse(input).unwrap();
        assert_eq!(data.files.len(), 0);
    }

    #[test]
    fn test_parse_cobertura_malformed() {
        // Malformed XML should produce a meaningful error with position info.
        let input = include_bytes!("../../tests/fixtures/malformed_cobertura.xml");
        let result = parse(input);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        // Error should mention position
        assert!(
            err_msg.contains("position"),
            "Error should contain position info: {err_msg}",
        );
    }
}
