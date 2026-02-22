pub mod cobertura;
pub mod gocover;
pub mod istanbul;
pub mod jacoco;
pub mod lcov;

use std::io::BufRead;
use std::path::Path;

use anyhow::Result;
use quick_xml::events::BytesStart;
use quick_xml::reader::Reader;

use crate::model::FileCoverage;

/// Parser for a specific coverage format.
pub trait CoverageParser {
    /// The format this parser handles.
    fn format(&self) -> Format;

    /// Whether this parser can handle the given file, based on its path and
    /// content. Implementations should be cheap — only inspect the extension
    /// and/or the first few KB of content.
    fn can_parse(&self, path: &Path, content: &[u8]) -> bool;

    /// Streaming parse from a buffered reader: calls `emit` once per source
    /// file instead of collecting everything into a single `CoverageData`.
    fn parse_streaming(
        &self,
        reader: &mut dyn BufRead,
        emit: &mut dyn FnMut(FileCoverage) -> Result<()>,
    ) -> Result<()>;
}

// ── Shared XML helpers used by cobertura & jacoco parsers ──────────

/// Peek at the first 4 KiB of content as a string for format detection.
pub(crate) fn sniff_head(content: &[u8]) -> std::borrow::Cow<'_, str> {
    let n = content.len().min(4096);
    String::from_utf8_lossy(&content[..n])
}

/// Whether the given text snippet looks like XML.
pub(crate) fn looks_like_xml(head: &str) -> bool {
    head.contains("<?xml") || head.trim_start().starts_with('<')
}

/// Extract a single attribute value from an XML element.
pub(crate) fn get_attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    let attr = e.try_get_attribute(name).ok()??;
    attr.unescape_value().ok().map(|v| v.into_owned())
}

/// Create a configured XML reader from a buffered source.
pub(crate) fn xml_reader<R: BufRead>(input: R) -> Reader<R> {
    let mut reader = Reader::from_reader(input);
    reader.trim_text(true);
    reader
}

/// Map a quick_xml error to an anyhow error with buffer position context.
pub(crate) fn xml_err<R>(e: quick_xml::Error, reader: &Reader<R>) -> anyhow::Error {
    let pos = reader.buffer_position();
    anyhow::anyhow!("XML parse error at position {pos}: {e}")
}

/// Supported coverage formats, used for the `--format` CLI override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cobertura,
    Gocover,
    Istanbul,
    Jacoco,
    Lcov,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Cobertura => f.write_str("cobertura"),
            Format::Gocover => f.write_str("gocover"),
            Format::Istanbul => f.write_str("istanbul"),
            Format::Jacoco => f.write_str("jacoco"),
            Format::Lcov => f.write_str("lcov"),
        }
    }
}

impl std::str::FromStr for Format {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cobertura" => Ok(Format::Cobertura),
            "gocover" | "go" => Ok(Format::Gocover),
            "istanbul" | "nyc" => Ok(Format::Istanbul),
            "jacoco" => Ok(Format::Jacoco),
            "lcov" => Ok(Format::Lcov),
            _ => Err(anyhow::anyhow!(
                "Unknown format: '{s}'. Supported: cobertura, gocover, istanbul, jacoco, lcov"
            )),
        }
    }
}

/// All registered parsers, in detection priority order.
///
/// LCOV is checked first because its content markers are very specific
/// (lines starting with `SF:`, `DA:`, etc.) so false positives are unlikely.
/// Go cover is next — its `mode:` header and `.go:` block patterns are
/// equally distinctive and won't collide with LCOV or XML formats.
/// Istanbul is checked before the XML parsers because its JSON-based
/// `statementMap`/`s` markers are very specific and won't collide.
/// JaCoCo is checked before Cobertura since both are XML but JaCoCo's
/// `<report` + `jacoco`/`<package` markers are more specific than
/// Cobertura's `<coverage`.
pub fn all() -> Vec<Box<dyn CoverageParser>> {
    vec![
        Box::new(lcov::LcovParser),
        Box::new(gocover::GocoverParser),
        Box::new(istanbul::IstanbulParser),
        Box::new(jacoco::JacocoParser),
        Box::new(cobertura::CoberturaParser),
    ]
}

/// Detect the format and return the matching parser, or `None`.
pub fn detect(path: &Path, content: &[u8]) -> Option<Box<dyn CoverageParser>> {
    all().into_iter().find(|p| p.can_parse(path, content))
}

/// Get the appropriate parser for an explicit format name.
pub fn for_format(format: Format) -> Box<dyn CoverageParser> {
    match format {
        Format::Cobertura => Box::new(cobertura::CoberturaParser),
        Format::Gocover => Box::new(gocover::GocoverParser),
        Format::Istanbul => Box::new(istanbul::IstanbulParser),
        Format::Jacoco => Box::new(jacoco::JacocoParser),
        Format::Lcov => Box::new(lcov::LcovParser),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_lcov_by_extension() {
        let parser = detect(Path::new("coverage.info"), b"").unwrap();
        assert_eq!(parser.format(), Format::Lcov);

        let parser = detect(Path::new("coverage.lcov"), b"").unwrap();
        assert_eq!(parser.format(), Format::Lcov);
    }

    #[test]
    fn test_detect_lcov_by_content() {
        let content = b"TN:test\nSF:/src/lib.rs\nDA:1,5\nend_of_record\n";
        let parser = detect(Path::new("coverage.txt"), content).unwrap();
        assert_eq!(parser.format(), Format::Lcov);
    }

    #[test]
    fn test_detect_jacoco_by_content() {
        let content =
            b"<?xml version=\"1.0\"?>\n<report name=\"test\"><package name=\"com/example\">";
        let parser = detect(Path::new("jacoco.xml"), content).unwrap();
        assert_eq!(parser.format(), Format::Jacoco);
    }

    #[test]
    fn test_detect_jacoco_by_doctype() {
        let content =
            b"<?xml version=\"1.0\"?><!DOCTYPE report PUBLIC \"-//JACOCO//DTD Report 1.1//EN\" \"report.dtd\"><report name=\"test\">";
        let parser = detect(Path::new("report.xml"), content).unwrap();
        assert_eq!(parser.format(), Format::Jacoco);
    }

    #[test]
    fn test_detect_cobertura_by_content() {
        let content = b"<?xml version=\"1.0\"?>\n<coverage version=\"1.0\">";
        let parser = detect(Path::new("coverage.xml"), content).unwrap();
        assert_eq!(parser.format(), Format::Cobertura);
    }

    #[test]
    fn test_detect_gocover_by_extension() {
        let parser = detect(Path::new("coverage.coverprofile"), b"").unwrap();
        assert_eq!(parser.format(), Format::Gocover);

        let parser = detect(Path::new("coverage.gocov"), b"").unwrap();
        assert_eq!(parser.format(), Format::Gocover);
    }

    #[test]
    fn test_detect_gocover_by_content() {
        let content = b"mode: count\ngithub.com/user/repo/main.go:10.1,20.5 3 1\n";
        let parser = detect(Path::new("coverage.out"), content).unwrap();
        assert_eq!(parser.format(), Format::Gocover);
    }

    #[test]
    fn test_detect_istanbul_by_filename() {
        let parser = detect(Path::new("coverage-final.json"), b"").unwrap();
        assert_eq!(parser.format(), Format::Istanbul);
    }

    #[test]
    fn test_detect_istanbul_by_content() {
        let content = br#"{ "/src/lib.js": { "statementMap": { "0": { "start": { "line": 1 } } }, "s": { "0": 1 }, "fnMap": {}, "f": {} } }"#;
        let parser = detect(Path::new("coverage.json"), content).unwrap();
        assert_eq!(parser.format(), Format::Istanbul);
    }

    #[test]
    fn test_detect_unknown() {
        assert!(detect(Path::new("random.dat"), b"hello world").is_none());
    }
}
