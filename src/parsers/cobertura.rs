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
use std::str;
use std::sync::LazyLock;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use regex::Regex;

/// Pre-compiled regex for condition-coverage attributes like "75% (3/4)".
static BRANCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d+)/(\d+)\)").unwrap());

use crate::error::{CovrsError, Result};
use crate::model::*;
use crate::parsers::Parser;

pub struct CoberturaParser;

impl Parser for CoberturaParser {
    fn parse(&self, input: &[u8]) -> Result<CoverageData> {
        parse_cobertura(input)
    }
}

fn parse_cobertura(input: &[u8]) -> Result<CoverageData> {
    let mut reader = Reader::from_reader(input);
    reader.trim_text(true);

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
            Err(e) => return Err(CovrsError::Xml(e)),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local_name = e.name();
                let local = local_name.as_ref().to_vec();

                match local.as_slice() {
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
                        let attrs = attr_map(e);
                        if let Some(filename) = attrs.get("filename") {
                            let path = resolve_source_path(filename, &sources);
                            current_file = Some(FileCoverage::new(path));
                            branch_indices.clear();
                            line_index_map.clear();
                        }
                    }
                    b"method" => {
                        let attrs = attr_map(e);
                        in_method = true;
                        current_method_name = attrs.get("name").cloned();
                        method_hit = false;
                        method_start_line = None;
                    }
                    b"line" => {
                        let attrs = attr_map(e);
                        if let Some(file) = current_file.as_mut() {
                            if let Some(number_str) = attrs.get("number") {
                                if let Ok(line_number) = number_str.parse::<u32>() {
                                    let hit_count = attrs
                                        .get("hits")
                                        .and_then(|h| h.parse::<u64>().ok())
                                        .unwrap_or(0);

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
                                    let is_branch = attrs
                                        .get("branch")
                                        .map(|v| v == "true")
                                        .unwrap_or(false);

                                    if is_branch && !branch_indices.contains_key(&line_number) {
                                        if let Some(cond) = attrs.get("condition-coverage") {
                                            if let Some(caps) = branch_re.captures(cond) {
                                                let covered: u32 =
                                                    caps[1].parse().unwrap_or(0);
                                                let total: u32 =
                                                    caps[2].parse().unwrap_or(0);

                                                for i in 0..total {
                                                    // Cobertura's condition-coverage only tells
                                                    // us how many branches were taken, not per-
                                                    // branch execution counts. Use 1 for covered
                                                    // arms and 0 for uncovered.
                                                    let branch_hit: u64 = if i < covered {
                                                        1
                                                    } else {
                                                        0
                                                    };
                                                    let idx = branch_indices
                                                        .entry(line_number)
                                                        .or_insert(0);
                                                    file.branches.push(BranchCoverage {
                                                        line_number,
                                                        branch_index: *idx,
                                                        hit_count: branch_hit,
                                                    });
                                                    *branch_indices
                                                        .get_mut(&line_number)
                                                        .unwrap() += 1;
                                                }
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
            Ok(Event::End(ref e)) => {
                let local_name = e.name();
                let local = local_name.as_ref().to_vec();
                match local.as_slice() {
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
                }
            }
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
            return format!("{}/{}", base, filename);
        }
    }
    filename.to_string()
}

/// Extract attributes from an XML element into a HashMap.
fn attr_map(e: &quick_xml::events::BytesStart) -> HashMap<String, String> {
    e.attributes()
        .filter_map(|a| {
            let attr = a.ok()?;
            let key = str::from_utf8(attr.key.local_name().into_inner())
                .ok()?
                .to_string();
            let value = attr.unescape_value().ok()?.to_string();
            Some((key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cobertura() {
        let input = include_bytes!("../../tests/fixtures/sample_cobertura.xml");
        let parser = CoberturaParser;
        let data = parser.parse(input).unwrap();

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
        let parser = CoberturaParser;
        let data = parser.parse(input).unwrap();

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
        let parser = CoberturaParser;
        let data = parser.parse(input).unwrap();

        assert_eq!(data.files.len(), 1);
        // Should use the first non-empty source as prefix, not the empty one.
        assert_eq!(data.files[0].path, "/home/user/project/src/app.py");
    }

    #[test]
    fn test_parse_cobertura_no_sources() {
        let input = include_bytes!("../../tests/fixtures/cobertura_no_sources.xml");
        let parser = CoberturaParser;
        let data = parser.parse(input).unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].path, "src/f.rs");
    }
}
